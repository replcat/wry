use super::conversion::*;
use cookie::time::error;
use dispatch2::DispatchQueue;
use objc2::{
  rc::Retained,
  runtime::{AnyObject, ProtocolObject},
  ClassType,
};
use objc2_foundation::*;
use objc2_web_kit::WKURLSchemeTask;
use std::{borrow::Cow, cell::RefCell, collections::HashMap, sync::mpsc};

#[derive(thiserror::Error, Debug)]
pub enum TaskError {
  #[error("task was externally invalidated")]
  TaskInvalidated,
  #[error("attempted to finish stream or send data before responding")]
  ResponseNotSent,
  #[error("receiver dropped or queue blocked indefinitely")]
  QueueSyncFailed,
}

pub(super) fn task_id(task: &ProtocolObject<dyn WKURLSchemeTask>) -> usize {
  task as *const _ as usize
}

/// Thread-safe handle for an underlying `WKURLSchemeTask`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SafeTask {
  task_id: usize,
}

unsafe impl Send for SafeTask {}
unsafe impl Sync for SafeTask {}

thread_local! {
  static ACTIVE_TASKS: RefCell<HashMap<usize, TaskState>> = RefCell::new(HashMap::new());
}

#[derive(Debug)]
pub(super) struct TaskState {
  pub task: Retained<ProtocolObject<dyn WKURLSchemeTask>>,
  pub is_valid: bool,
  pub response_sent: bool,
}

impl SafeTask {
  pub(super) fn new(task: &ProtocolObject<dyn WKURLSchemeTask>, _mtm: MainThreadMarker) -> Self {
    let task_id = task_id(task);

    ACTIVE_TASKS.with_borrow_mut(|cell| {
      cell.insert(
        task_id,
        TaskState {
          task: unsafe { Retained::retain(task as *const _ as *mut _) }.unwrap(),
          is_valid: true,
          response_sent: false,
        },
      )
    });

    Self { task_id }
  }

  pub fn stop(task: &ProtocolObject<dyn WKURLSchemeTask>, _mtm: MainThreadMarker) {
    ACTIVE_TASKS.with_borrow_mut(|cell| {
      if let Some(state) = cell.get_mut(&task_id(task)) {
        state.is_valid = false;
      }
    });
  }

  pub fn id(&self) -> usize {
    self.task_id
  }

  pub(super) fn dispatch_sync_on_main<F, R>(&self, f: F) -> Result<R, TaskError>
  where
    F: Send + FnOnce(MainThreadMarker) -> Result<R, TaskError>,
    R: Send + 'static,
  {
    if let Some(mtm) = MainThreadMarker::new() {
      return f(mtm); // already on main thread
    }

    let (tx, rx) = mpsc::channel();

    DispatchQueue::main().exec_sync(move || {
      let mtm = MainThreadMarker::new().unwrap();
      let _ = tx.send(f(mtm));
    });

    rx.recv().map_err(|_| TaskError::QueueSyncFailed)?
  }
}

impl crate::PlatformAgnosticStreamHandle for SafeTask {
  type Error = TaskError;

  fn send_response(&self, response: http::Response<()>) -> Result<(), Self::Error> {
    self.dispatch_sync_on_main(move |_mtm| {
      ACTIVE_TASKS.with_borrow_mut(|cell| match cell.get_mut(&self.task_id) {
        Some(state) if state.is_valid => Ok(unsafe {
          let response = response.into_objc(state.task.request());
          state.task.didReceiveResponse(&response);
          state.response_sent = true;
        }),
        _ => Err(TaskError::TaskInvalidated),
      })
    })
  }

  fn send_data(&self, data: Cow<'static, [u8]>) -> Result<(), Self::Error> {
    self.dispatch_sync_on_main(move |_mtm| {
      ACTIVE_TASKS.with_borrow_mut(|cell| match cell.get_mut(&self.task_id) {
        None => Err(TaskError::TaskInvalidated),
        Some(state) if !state.response_sent => Err(TaskError::ResponseNotSent),
        Some(state) if !state.is_valid => Err(TaskError::TaskInvalidated),
        Some(state) => Ok(unsafe { state.task.didReceiveData(&NSData::with_bytes(&data)) }),
      })
    })
  }

  fn finish(&self) -> Result<(), Self::Error> {
    self.dispatch_sync_on_main(move |_mtm| {
      ACTIVE_TASKS.with_borrow_mut(|cell| match cell.get_mut(&self.task_id) {
        None => Err(TaskError::TaskInvalidated),
        Some(state) if !state.response_sent => Err(TaskError::ResponseNotSent),
        Some(state) if !state.is_valid => Ok(()), // already invalidated
        Some(state) => Ok(unsafe { state.task.didFinish() }),
      })
    })
  }

  fn fail(&self, error_message: String) -> Result<(), Self::Error> {
    self.dispatch_sync_on_main(move |_mtm| {
      ACTIVE_TASKS.with_borrow_mut(|cell| match cell.get_mut(&self.task_id) {
        None => Err(TaskError::TaskInvalidated),
        Some(state) if !state.response_sent => Err(TaskError::ResponseNotSent),
        Some(state) if !state.is_valid => Ok(()), // already invalidated
        Some(state) => Ok(unsafe {
          let userinfo = NSMutableDictionary::<NSString, AnyObject>::new();
          let message = NSString::from_str(&error_message);
          userinfo.insert(NSLocalizedDescriptionKey, &*message as &AnyObject);

          let domain = NSString::from_str(env!("CARGO_PKG_NAME"));
          let error = NSError::errorWithDomain_code_userInfo(&domain, 1, Some(&*userinfo.copy()));
          state.task.didFailWithError(&error);
        }),
      })
    })
  }
}
