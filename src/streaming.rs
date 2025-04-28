use crate::{wkwebview, RequestAsyncResponder};
use http::Response;
use std::{
  borrow::Cow,
  sync::{Arc, Mutex},
};

pub(crate) trait PlatformAgnosticStreamHandle: Send + Sync + std::fmt::Debug {
  type Error: std::error::Error + Send + Sync + 'static;
  fn send_response(&self, response: Response<()>) -> std::result::Result<(), Self::Error>;
  fn send_data(&self, data: Cow<'static, [u8]>) -> std::result::Result<(), Self::Error>;
  fn finish(&self) -> std::result::Result<(), Self::Error>;
  fn fail(&self, error_message: String) -> std::result::Result<(), Self::Error>;
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) type PlatformStreamHandle = wkwebview::SafeTask;

#[must_use = "by completing with `.finish()` or `.fail()`"]
pub struct StreamHandle {
  inner: Arc<PlatformStreamHandle>,
  completed: Arc<Mutex<bool>>,
}

unsafe impl Send for StreamHandle {}

impl StreamHandle {
  pub(crate) fn new(context: RequestAsyncResponder, headers: Response<()>) -> crate::Result<Self> {
    Ok(Self {
      inner: Arc::new((context.get_platform_handle)(headers)?),
      completed: Arc::new(Mutex::new(false)),
    })
  }

  pub fn send_chunk<T: Into<Cow<'static, [u8]>>>(self, chunk: T) -> Self {
    self
      .inner
      .send_data(chunk.into())
      .unwrap_or_else(|err| panic!("internal error: {err}"));
    self
  }

  pub fn finish(self) {
    let mut completed = self.completed.lock().unwrap();
    *completed = true;
    self
      .inner
      .finish()
      .unwrap_or_else(|err| panic!("internal error: {err}"));
  }

  pub fn fail(self, error_message: String) {
    let mut completed = self.completed.lock().unwrap();
    *completed = true;
    self
      .inner
      .fail(error_message)
      .unwrap_or_else(|err| panic!("internal error: {err}"));
  }
}

impl Drop for StreamHandle {
  fn drop(&mut self) {
    if !*self.completed.lock().unwrap() {
      #[cfg(feature = "tracing")]
      tracing::warn!("`StreamHandle` dropped without completing with `.finish()` or `.fail()`");
    }
  }
}
