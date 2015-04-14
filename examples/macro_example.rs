#[macro_use] extern crate nickel;
extern crate url;
extern crate regex;
extern crate rustc_serialize;

use std::io::Write;
use nickel::status::StatusCode::{self, NotFound};
use nickel::{
    Nickel, NickelError, Continue, Halt, Request, Response,
    QueryString, JsonBody, StaticFilesHandler, MiddlewareResult, HttpRouter, Action
};
use regex::Regex;

#[derive(RustcDecodable, RustcEncodable)]
struct Person {
    firstname: String,
    lastname:  String,
}

//this is an example middleware function that just logs each request
fn logger<'a>(request: &mut Request, response: Response<'a>) -> MiddlewareResult<'a> {
    println!("logging request: {:?}", request.origin.uri);
    Ok(Continue(response))
}

//this is how to overwrite the default error handler to handle 404 cases with a custom view
fn custom_404<'a>(err: &mut NickelError, _req: &mut Request) -> Action {
    if let Some(ref mut res) = err.stream {
        if res.status() == NotFound {
            let _ = res.write_all(b"<h1>Call the police!</h1>");
            return Halt(())
        }
    }

    Continue(())
}

fn main() {
    let mut server = Nickel::new();

    // middleware is optional and can be registered with `utilize`
    server.utilize(middleware!(@logger));

    // go to http://localhost:6767/thoughtram_logo_brain.png to see static file serving in action
    server.utilize(StaticFilesHandler::new("examples/assets/"));

    let hello_regex = Regex::new("/hello/(?P<name>[a-zA-Z]+)").unwrap();

    // The return type for a route can be anything that implements `ResponseFinalizer`
    server.utilize(router!(
        // go to http://localhost:6767/user/4711 to see this route in action
        get "/user/:userid" => |request, response| {
            // returning a String
            format!("This is user: {}", request.param("userid"))
        }

        // go to http://localhost:6767/no_alloc/4711 to see this route in action
        get "/no_alloc/:userid" => |request, response| {
            // returning a slice of T where T: Display
            &["This is user: ", request.param("userid")][..]
        }

        // go to http://localhost:6767/bar to see this route in action
        get "/bar" => |request, response| {
            // returning a http status code and a static string
            (200u16, "This is the /bar handler")
        }

        // go to http://localhost:6767/hello/moomah to see this route in action
        get hello_regex => |request, response| {
            format!("Hello {}", request.param("name"))
        }

        // FIXME
        // // go to http://localhost:6767/redirect to see this route in action
        // get "/redirect" => |request, response| {
        //     use http::headers::response::Header::Location;
        //     let root = url::Url::parse("http://www.rust-lang.org/").unwrap();
        //     // returning a typed http status, a response body and some additional headers
        //     (StatusCode::TemporaryRedirect, "Redirecting you to 'rust-lang.org'", vec![Location(root)])
        // }

        // go to http://localhost:6767/private to see this route in action
        get "/private" => |request, response| {
            // returning a typed http status and a response body
            (StatusCode::Unauthorized, "This is a private place")
        }

        // go to http://localhost:6767/some/crazy/route to see this route in action
        get "/some/*/route" => |request, response| {
            // returning a static string
            "This matches /some/crazy/route but not /some/super/crazy/route"
        }

        // go to http://localhost:6767/some/crazy/route to see this route in action
        get "/a/**/route" => |request, response| {
            "This matches /a/crazy/route and also /a/super/crazy/route"
        }

        // try it with curl
        // curl 'http://localhost:6767/a/post/request' -H 'Content-Type: application/json;charset=UTF-8'  --data-binary $'{ "firstname": "John","lastname": "Connor" }'
        post "/a/post/request" => |request, response| {
            let person = request.json_as::<Person>().unwrap();
            format!("Hello {} {}", person.firstname, person.lastname)
        }

        // try calling http://localhost:6767/query?foo=bar
        get "/query" => |request, response| {
            let text = format!("Your foo values in the query string are: {:?}",
                               request.query("foo", "This is only a default value!"));
            text
        }
    ));

    // issue #20178
    let custom_handler: fn(&mut NickelError, &mut Request) -> Action = custom_404;

    server.handle_error(custom_handler);

    println!("Running server!");
    server.listen("127.0.0.1:6767");
}
