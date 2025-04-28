use super::{conversion::*, task::SafeTask};
use crate::{PlatformAgnosticStreamHandle, RequestAsyncResponder, WebViewId};
use objc2::{exception::*, rc::Retained, runtime::*, *};
use objc2_foundation::NSString;
use objc2_web_kit::{WKURLSchemeHandler, WKURLSchemeTask, WKWebView, WKWebViewConfiguration};
use std::panic::AssertUnwindSafe;

#[derive(thiserror::Error, Debug)]
pub enum HandlerError {
  #[error("handler creation failed with caught exception: {0}")]
  Caught(#[from] Retained<Exception>),
  #[error("handler creation failed with an empty exception")]
  CaughtNil,
}

type Implementation = Box<dyn Fn(WebViewId, http::Request<Vec<u8>>, RequestAsyncResponder)>;

pub(crate) struct Ivars {
  webview_id: String,
  implementation: Implementation,
}

impl Handler {
  pub(crate) fn try_attach(
    webview_config: &Retained<WKWebViewConfiguration>,
    webview_id: WebViewId,
    protocol: &str,
    implementation: Implementation,
  ) -> Result<Retained<Self>, HandlerError> {
    exception::catch(AssertUnwindSafe(|| {
      let mtm = MainThreadMarker::new().unwrap();

      let this = Self::alloc(mtm).set_ivars(Ivars {
        webview_id: webview_id.into(),
        implementation,
      });

      let handler: Retained<Self> = unsafe { msg_send![super(this), init] };

      unsafe {
        webview_config.setURLSchemeHandler_forURLScheme(
          Some(&ProtocolObject::from_retained(handler.retain())),
          &NSString::from_str(&protocol),
        )
      }

      return handler;
    }))
    .map_err(|caught| caught.map_or(HandlerError::CaughtNil, HandlerError::from))
  }
}

define_class!(
  #[unsafe(super(NSObject))]
  #[thread_kind = MainThreadOnly]
  #[name = "Handler"]
  #[ivars = Ivars]
  pub(crate) struct Handler;

  unsafe impl NSObjectProtocol for Handler {}

  unsafe impl WKURLSchemeHandler for Handler {
    #[unsafe(method(webView:startURLSchemeTask:))]
    fn start(&self, _: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
      let mtm = MainThreadMarker::new().unwrap();
      let safe_task = SafeTask::new(task, mtm);
      let ivars = self.ivars();

      (ivars.implementation)(
        &ivars.webview_id,
        task.into_http(),
        RequestAsyncResponder {
          responder: Box::new(move |response| {
            let (parts, body) = response.into_parts();
            safe_task
              .send_response(http::Response::from_parts(parts, ()))
              .unwrap();
            safe_task.send_data(body).unwrap();
            safe_task.finish().unwrap();
          }),

          get_platform_handle: Box::new(move |response| {
            safe_task.send_response(response)?;
            Ok(safe_task)
          }),
        },
      )
    }

    #[unsafe(method(webView:stopURLSchemeTask:))]
    fn stop(&self, _: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
      let mtm = MainThreadMarker::new().unwrap();
      SafeTask::stop(task, mtm);
    }
  }
);
