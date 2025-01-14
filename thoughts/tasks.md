Table Stakes
- [x] Persistent images, by way of sqlite files
- [x] Importing modules
    - [x] Convert imported globals to exported globals w/ defaults
    - [x] Convert imported memories to equivalent exported memory
- [x] Instantiating instance
    - [x] Persist mutated globals after execution session
    - [x] Persist mutated memory segments after execution session
- [x] Define messaging interface
    - [X] Work it out.. see `web_server.md`
    - [ ] Consider if something like symlinks + faux directories might be a good configuration story
    ```
    For example, suppose we instantiate an HTTP server at 
    `/http`
    It might create sub directories like 
    `/http.sites/` and `/http.certs/` 
    You could create a link to an instance which actually handles requests for a particular site at
    `/http.sites/acme.com` which points to `/acme/some.instance`
    ```
- [ ] Define error catching & telemetry logic in `telemetry.md`
    - [ ] Examine WASM fault semantics
    - [ ] Examine Rust & WASM panic semantics
    - [ ] How should telemetry be exposed to an image?
- [ ] Provide ‘native’ modules from Othismo, not .wasm files from filesystem
    - [ ] `othismo.console` module, which echoes messages to console
    - [ ] `othismo.namespace` module, basically handles what would’ve been called sys calls.  Like for enumerating the namespace.
    - [ ] `othismo.http` module, simple HTTP pass thru
    - [ ] `othismo.files` module, simple blob storage of files imported via CLI, but exposed via namespace
- [ ] CLI update to send arbitrary-ish messages
    - [ ] Make use of foo.bar notation to craft nested messages via CLI
    - [ ] Make use of templates to allow easy for crafting deeply nested messages via CLI
    - [ ] Allow for templates to be found in local file system, or in the namespace of the image at hand

Web Server of Files

- [ ] Create native http module
    - [ ] Converts HTTP requests to messages, sent to another instance
    - [ ] Responses 