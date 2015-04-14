#[macro_use] extern crate nickel;

use nickel::{Nickel, Request, Response, HttpRouter, MiddlewareResult};
use std::collections::HashMap;

fn main() {
    let mut server = Nickel::new();

    fn handler<'a>(_: &mut Request, res: Response<'a>) -> MiddlewareResult<'a> {
        let mut data = HashMap::<&str, &str>::new();
        data.insert("name", "user");
        res.render("examples/assets/template.tpl", &data)
    }

    server.get("/", middleware!(@handler));

    server.listen("127.0.0.1:6767");
}
