use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::*;
use objc2_web_kit::WKURLSchemeTask;
use std::{ptr::NonNull, str::FromStr};

pub(crate) trait IntoHttpExt {
  fn into_http(&self) -> http::Request<Vec<u8>>;
}

impl IntoHttpExt for ProtocolObject<dyn WKURLSchemeTask> {
  fn into_http(&self) -> http::Request<Vec<u8>> {
    unsafe { self.request().into_http() }
  }
}

impl IntoHttpExt for Retained<NSURLRequest> {
  fn into_http(&self) -> http::Request<Vec<u8>> {
    let mut http_request = http::Request::builder();

    unsafe {
      if let Some(method) = self.HTTPMethod() {
        http_request = http_request.method(
          // SAFETY: The conditions in which this would fail are impossible to
          // create in practice -- i.e. the webview stops the invalid requests.
          http::Method::from_str(&method.to_string()).unwrap(),
        );
      }

      if let Some(url) = self.URL().and_then(|url| url.absoluteString()) {
        http_request = http_request.uri(url.to_string());
      }

      if let Some(headers) = self.allHTTPHeaderFields() {
        for key in headers.keyEnumerator().iter() {
          for value in headers.objectForKey(&*key).iter() {
            http_request = http_request.header(key.to_string(), value.to_string());
          }
        }
      }

      if let Some(body) = self.HTTPBody() {
        return http_request.body(body.to_vec()).unwrap();
      }

      if let Some(body_stream) = self.HTTPBodyStream() {
        let mut body = Vec::new();
        let mut buffer = vec![0u8; 1024];
        body_stream.open();

        while body_stream.hasBytesAvailable() {
          let bytes_read =
            body_stream.read_maxLength(NonNull::new(buffer.as_mut_ptr()).unwrap(), buffer.len());

          if bytes_read > 0 {
            body.extend_from_slice(&buffer[..bytes_read as usize]);
          }
        }

        body_stream.close();
        return http_request.body(body).unwrap();
      }

      http_request.body(Vec::new()).unwrap()
    }
  }
}

#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
  use super::*;

  #[test]
  fn test_into_http_from_simple_get_request() {
    let url = "proto://cool/resource";

    let nsrequest = unsafe {
      let nsurl = NSURL::URLWithString(&*NSString::from_str(url)).unwrap();
      NSURLRequest::requestWithURL(&*nsurl)
    };

    let http_request = nsrequest.into_http();

    assert_eq!(http_request.method(), "GET");
    assert_eq!(http_request.uri().to_string(), url);
    assert!(http_request.headers().is_empty());
    assert!(http_request.body().is_empty());
  }

  #[test]
  fn test_into_http_from_busy_post_request() {
    let url = "proto://cool/resource";
    let method = "POST";
    let body = b"beans";

    let nsrequest = unsafe {
      let nsurl = NSURL::URLWithString(&*NSString::from_str(url)).unwrap();
      let nsrequest = NSMutableURLRequest::new();
      nsrequest.setHTTPMethod(&*NSString::from_str(method));
      nsrequest.setURL(Some(&*nsurl));
      nsrequest.addValue_forHTTPHeaderField(
        &*NSString::from_str("text/plain"),
        &*NSString::from_str("content-type"),
      );
      nsrequest.setHTTPBody(Some(&*NSData::with_bytes(body)));
      nsrequest.copy()
    };

    let http_request = nsrequest.into_http();

    assert_eq!(http_request.method(), "POST");
    assert_eq!(http_request.uri().to_string(), url);
    assert_eq!(
      http_request.headers().get("content-type").unwrap(),
      "text/plain"
    );
    assert_eq!(*http_request.body(), body.as_ref());
  }

  #[test]
  fn test_into_http_with_multiple_headers() {
    let url = "proto://cool/resource";
    let method = "GET";

    let nsrequest = unsafe {
      let nsurl = NSURL::URLWithString(&*NSString::from_str(url)).unwrap();
      let nsrequest = NSMutableURLRequest::new();
      nsrequest.setHTTPMethod(&*NSString::from_str(method));
      nsrequest.setURL(Some(&*nsurl));
      nsrequest.addValue_forHTTPHeaderField(
        &*NSString::from_str("cool"),
        &*NSString::from_str("header-one"),
      );
      nsrequest.addValue_forHTTPHeaderField(
        &*NSString::from_str("beans"),
        &*NSString::from_str("header-two"),
      );
      nsrequest.copy()
    };

    let http_request = nsrequest.into_http();

    assert_eq!(http_request.method(), "GET");
    assert_eq!(http_request.uri().to_string(), url);
    assert_eq!(http_request.headers().len(), 2);
    assert_eq!(http_request.headers().get("header-one").unwrap(), "cool");
    assert_eq!(http_request.headers().get("header-two").unwrap(), "beans");
    assert!(http_request.body().is_empty());
  }

  #[test]
  fn test_intp_http_from_empty_stream() {
    let url = "proto://cool/resource";
    let method = "POST";
    let body = &[];

    let nsrequest = unsafe {
      let nsurl = NSURL::URLWithString(&*NSString::from_str(url)).unwrap();
      let nsrequest = NSMutableURLRequest::new();
      nsrequest.setHTTPMethod(&*NSString::from_str(method));
      nsrequest.setURL(Some(&*nsurl));
      nsrequest.addValue_forHTTPHeaderField(
        &*NSString::from_str("text/plain"),
        &*NSString::from_str("content-type"),
      );
      let nsinputstream = NSInputStream::inputStreamWithData(&*NSData::with_bytes(body)).unwrap();
      nsrequest.setHTTPBodyStream(Some(&*nsinputstream));
      nsrequest.copy()
    };

    let http_request = nsrequest.into_http();

    assert_eq!(http_request.method(), "POST");
    assert_eq!(http_request.uri().to_string(), url);
    assert_eq!(
      http_request.headers().get("content-type").unwrap(),
      "text/plain"
    );
    assert!(http_request.body().is_empty());
  }

  #[test]
  fn test_into_http_from_misaligned_stream() {
    let url = "proto://cool/resource";
    let method = "POST";
    let body_size = 1024 * 3 + 512;
    let body: Vec<u8> = b"beans".iter().cycle().take(body_size).copied().collect();
    let body_slice = body.as_slice();

    let nsrequest = unsafe {
      let nsurl = NSURL::URLWithString(&*NSString::from_str(url)).unwrap();
      let nsrequest = NSMutableURLRequest::new();
      nsrequest.setHTTPMethod(&*NSString::from_str(method));
      nsrequest.setURL(Some(&*nsurl));
      nsrequest.addValue_forHTTPHeaderField(
        &*NSString::from_str("text/plain"),
        &*NSString::from_str("content-type"),
      );
      let nsinputstream =
        NSInputStream::inputStreamWithData(&*NSData::with_bytes(body_slice)).unwrap();
      nsrequest.setHTTPBodyStream(Some(&*nsinputstream));
      nsrequest.copy()
    };

    let http_request = nsrequest.into_http();

    assert_eq!(http_request.method(), "POST");
    assert_eq!(http_request.uri().to_string(), url);
    assert_eq!(
      http_request.headers().get("content-type").unwrap(),
      "text/plain"
    );
    assert_eq!(*http_request.body(), body);
    assert_eq!(http_request.body().len(), body_size);
  }
}
