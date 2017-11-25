use util::*;

use futures::{Future, Stream};
use hyper::StatusCode;
use hyper::client::Response;

fn with_path<F>(path: &str, f: F) where F: FnOnce(Response) {
    run_example("mount", |port| {
        let url = format!("http://localhost:{}{}", port, path);
        let res = response_for(&url);
        f(res)
    })
}

#[test]
fn trims_the_prefix() {
    with_path("/test/foo", |res| {
        let s = read_body_to_string(res);
        assert_eq!(s, "Got request with uri = '/foo'");
    });

    with_path("/test/foo/bar.js", |res| {
        let s = read_body_to_string(res);
        assert_eq!(s, "Got request with uri = '/foo/bar.js'");
    })
}

#[test]
fn ignores_unmatched_prefixes() {
    with_path("/this_isnt_matched/foo", |res| {
        assert_eq!(res.status(), StatusCode::NotFound);
    })
}

#[test]
fn works_with_another_middleware() {
    with_path("/static/files/thoughtram_logo_brain.png", |res| {
        let status = res.status();
        let head = res.body().concat2().wait().unwrap();
        assert_eq!(status, StatusCode::Ok);
        assert!(!&head.is_empty(), "no data for thoughtram_logo_brain.png");
    });

    with_path("/static/files/nested/foo.js", |res| {
        let s = read_body_to_string(res);
        assert!(s.starts_with("function foo"), "unexpected response: {:?}", s);
    });
}

#[test]
fn fallthroughs_with_same_prefix() {
    // depends on `works_with_another_middleware` passing
    with_path("/static/files/a", |res| {
        let s = read_body_to_string(res);
        assert_eq!(s, "No static file with path '/a'!");
    });
}
