use http::header::CONTENT_LENGTH;
use objc2::{rc::Retained, *};
use objc2_foundation::*;

trait ContentLength {
  fn len(&self) -> usize;
}

impl ContentLength for () {
  fn len(&self) -> usize {
    0
  }
}

impl ContentLength for Vec<u8> {
  fn len(&self) -> usize {
    self.len()
  }
}

pub(crate) trait IntoObjcExt<B> {
  fn into_objc(&self, task: Retained<NSURLRequest>) -> Retained<NSHTTPURLResponse>;
}

impl<T: ContentLength> IntoObjcExt<T> for http::Response<T> {
  fn into_objc(&self, task_request: Retained<NSURLRequest>) -> Retained<NSHTTPURLResponse> {
    let headers = NSMutableDictionary::new();

    for (name, value) in self.headers().iter() {
      if let Ok(value) = value.to_str() {
        headers.insert(
          &*NSString::from_str(name.as_str()),
          &*NSString::from_str(value),
        );
      }
    }

    headers.insert(
      &*NSString::from_str(CONTENT_LENGTH.as_str()),
      &*NSString::from_str(&self.body().len().to_string()),
    );

    let response = NSHTTPURLResponse::alloc();
    let response = unsafe {
      NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
        response,
        &task_request.URL().unwrap_or_else(|| NSURL::new()),
        self.status().as_u16() as isize,
        Some(&NSString::from_str(&format!("{:#?}", self.version()))),
        Some(&headers),
      )
    };

    response.expect("failed to create NSHTTPURLResponse")
  }
}

#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
  use super::*;

  unsafe fn assert_header(headers: &Retained<NSDictionary>, key: &str, expected_value: &str) {
    let ns_key = NSString::from_str(key);

    let actual_value = headers
      .objectForKey(&*ns_key)
      .unwrap_or_else(|| panic!("header {key:?} not found"))
      .downcast::<NSString>()
      .unwrap_or_else(|_| panic!("header {key:?} value is not an NSString"));

    assert_eq!(
      actual_value.to_string(),
      expected_value,
      "header {key:?} value mismatch: expected {expected_value:?}, got {actual_value:?}"
    );
  }

  #[test]
  fn test_into_nsresponse_from_http_response() {
    let url = "proto://cool/resource";

    let http_response = http::Response::builder()
      .status(200)
      .header("content-type", "application/octet-stream")
      .body(vec![1, 2, 3, 4])
      .unwrap();

    let ns_response = unsafe {
      let nsurl = NSURL::URLWithString(&*NSString::from_str(url)).unwrap();
      let nsurl_request = NSURLRequest::requestWithURL(&nsurl);
      http_response.into_objc(nsurl_request)
    };

    unsafe {
      assert_eq!(ns_response.statusCode(), 200);
      assert_eq!(ns_response.expectedContentLength(), 4);
      assert_eq!(ns_response.URL().unwrap().relativeString().to_string(), url);

      let headers = ns_response.allHeaderFields();
      assert_header(&headers, "content-type", "application/octet-stream");
      assert_header(&headers, "content-length", "4");
      assert_eq!(headers.count(), 2); // content-type + content-length
    }
  }

  #[test]
  fn test_into_nsresponse_from_zero_length_http_response() {
    let url = "proto://cool/resource";

    let http_response = http::Response::builder()
      .status(200)
      .body(Vec::new())
      .unwrap();

    let ns_response = unsafe {
      let nsurl = NSURL::URLWithString(&*NSString::from_str(url)).unwrap();
      let nsurl_request = NSURLRequest::requestWithURL(&nsurl);
      http_response.into_objc(nsurl_request)
    };

    unsafe {
      assert_eq!(ns_response.statusCode(), 200);
      assert_eq!(ns_response.expectedContentLength(), 0);

      let headers = ns_response.allHeaderFields();
      assert_header(&headers, "content-length", "0");
      assert_eq!(headers.count(), 1);
    }
  }

  #[test]
  fn test_into_nsresponse_from_unit_http_response() {
    let url = "proto://cool/resource";

    let http_response_unit = http::Response::builder()
      .status(http::StatusCode::NO_CONTENT)
      .body(())
      .unwrap();

    let ns_response_unit = unsafe {
      let nsurl = NSURL::URLWithString(&*NSString::from_str(url)).unwrap();
      let nsurl_request = NSURLRequest::requestWithURL(&nsurl);
      http_response_unit.into_objc(nsurl_request)
    };

    unsafe {
      assert_eq!(ns_response_unit.statusCode(), 204);
      assert_eq!(ns_response_unit.expectedContentLength(), 0);

      let headers = ns_response_unit.allHeaderFields();
      assert_header(&headers, "content-length", "0");
      assert_eq!(headers.count(), 1);
    }
  }

  #[test]
  fn test_into_nsresponse_from_http_response_with_headers() {
    let url = "proto://cool/resource";

    let http_response = http::Response::builder()
      .status(200)
      .header("content-type", "text/plain")
      .header("header-one", "cool")
      .header("header-two", "beans")
      .body(vec![1, 2, 3, 4])
      .unwrap();

    let ns_response = unsafe {
      let nsurl = NSURL::URLWithString(&*NSString::from_str(url)).unwrap();
      let nsurl_request = NSURLRequest::requestWithURL(&nsurl);
      http_response.into_objc(nsurl_request)
    };

    unsafe {
      let headers = ns_response.allHeaderFields();
      assert_eq!(headers.count(), 4); // 3 + content-length
      assert_header(&headers, "header-one", "cool");
      assert_header(&headers, "header-two", "beans");
      assert_header(&headers, "content-type", "text/plain");
      assert_header(&headers, "content-length", "4");
      assert_eq!(ns_response.statusCode(), 200);
      assert_eq!(ns_response.expectedContentLength(), 4);
    }
  }

  #[test]
  fn test_into_nsresponse_from_http_response_with_invalid_utf8() {
    let url = "proto://cool/resource";

    let invalid_value = http::HeaderValue::from_bytes(b"\x80\x81\x82").unwrap();

    let http_response = http::Response::builder()
      .status(200)
      .header("valid-header", "valid-value")
      .header("invalid-header", invalid_value)
      .body(vec![1])
      .unwrap();

    let ns_response = unsafe {
      let nsurl = NSURL::URLWithString(&*NSString::from_str(url)).unwrap();
      let nsurl_request = NSURLRequest::requestWithURL(&nsurl);
      http_response.into_objc(nsurl_request)
    };

    unsafe {
      let headers = ns_response.allHeaderFields();
      assert_header(&headers, "valid-header", "valid-value");
      assert_header(&headers, "content-length", "1");

      assert!(headers
        .objectForKey(&*NSString::from_str("invalid-header"))
        .is_none()); // the invalid header is *not* present

      assert_eq!(headers.count(), 2); // i.e. valid-header + content-length
    }
  }
}
