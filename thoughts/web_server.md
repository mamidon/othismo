# Messaging Interface & PIDs

When configuring an image; we’re importing modules & instantiating instances.  Somehow we have to wire up these
Instances such that they can actually interact.

~~Option A: Direct configuration via messaging instances.  (Need to bind instances to names or IDs)~~

Option B: Massive manipulation of the namespace, ala Plan9.


### All Information Inside Messages

Instead of having multiple parameters, we encode everything inside the message via top level namespaces.  e.g.

```
{
  "othismo.send_to": "/namespace/foo",
  "acme.custom_message": {
    "foo": "bar"
  }
}
```


## Syscalls

### _allocate_message(handle: u32, message_length: u32, request_handle: u32) -> *const u8
Find and possibly allocate a buffer to receive a message with the specified handle.
The host must call this for every message it wishes to place in the inbox.

The request handle is optional, specify 0 if not relevant.
The host MUST place a message in the provided buffer; the guest will assume it's
available for processing during the next call to _run

### _run() -> ()
Processes all messages received since the last call to _run.  Yields control when all tasks 
are blocked awaiting I/O.

When this function returns, the host should not call this again until at least one message
has been sent to the guest.

### _send_message(bytes: *mut u8, length: u32) -> u32
Tells the host a message has been place in the inbox with a particular length at a particular location.
The return valie is that message's handle.  Any responses received will have their own handle, 
but will also specify this message's handle as the request_handle.

### _cast_message(bytes: *const u8, length: u32) -> u32
Has the same semantics as _send_message; except the runtime will not return any responses.

### _othismo_start() -> ()
Invoked once at initialization of the instance.

## Othismo built in messages for HTTP

### othismo
This message is included alongside other messages for specific routing.


```
{
    "othismo": {
        "send_to": "/some/thing",
        "reply_to": "/some/other/thing" // optional, to redirect responses elsewhere
    },
    "acme.custom_message": {
        // this will be sent to /some/thing
    }
}
```

### othismo.namespace.instantiate
Instantiates a new instance from a module in the image.

```
{
    "othismo.instantiate": {
        "module": "/some/namespace", // the module 
        "name": "/some/other/namespace" // location in the namespace of the new instance
    }
}
```

### othismo.namespace.import
Only available via the CLI.  Imports a webassembly module into the namespace.
By default, ./foo/fizzbuzz.wasm is imported to /othismo/modules/foo/fizzbuzz.
Providing `name` changes this destination.

```
{
    "othismo.import": {
        "file": "fizzbuzz.wasm", // the relative path of the file, including extension 
        "name": "/othismo/modules/fizzbuzz" // optional
    }
}
```

### othismo.namespace.list
Lists out all items in the namespace.  If `prefix` is provided,
the output is filtered by that prefix.

```
{
    "othismo.namespace.list": {
        "prefix": "/some/namespace/prefix" // optional
    }
}
```
The response is:
```
{
    "othismo.namespace.list.response": [
        "/something/in/the/namespace",
        "/some/other/thing"
    ]
}
```

### othismo.namespace.make_path
Create directories required for a path.
Directory names cannot conflict with existing objects.
```
{
    "othismo.namespace.make_path": {
        "path": "/some/fully/qualified/path"
    }
}
```

### othismo.namespace.sym_link
TODO -- redirects all messages at a particular path to another path
### othismo.namespace.mount
TODO -- redirects all namespace operations at or below /some/path to a particular instance
TODO -- how are mount & sym links different.. are they?

### othismo.http.request
Represents a received HTTP request.
```
{
    "othismo.http.request": {
        "host": "your_domain.com",
        "method": "GET" // POST etc
        "endpoint": "/some/relative/path",
        "query": {
            "key": "value"
        },
        "headers": {
            "key": "value"
        },
        "body": ... // bson bytes
    }
}
```


### othismo.http.request.response
Represents a response to a previous HTTP request.

```
{
    "othismo.http.request.response": {
        "status" 200,
        "headers": {
            "key": "value"
        },
        "body": ... // bson bytes
    }
}
```

### othismo.error
Anything can respond with an error
```
{
    "othismo.error": {
        "code": "unique_code",
        "message": "A human friendly message"
    }
}
```


## Configuring the server, with files

```
othismo new-image image
# othismo always exists in the namespace at /othismo
othismo image import-module othismo.http
othismo image import-module othismo.blobs

# import the actual custom code, which maps HTTP requests to blob responses
othismo image import-module prototype

# /modules contains othismo.http, othismo.blobs, prototype


# instantiate a web server
# also creates a folder /server/sites/ & /server/content/ where handlers or content for requests are found, hopefully
# also creates a folder /controller/content/ where 
othismo image instantiate-instance othismo.http server
othismo image instantiate-instance prototype controller
othismo image instantiate-instance othismo.blobs content

# import all files in ./www/ into blob storage
othismo image send-message /content cp=./www/
othismo image mount /content /server/content/othismo.com

# requests to the host othismo.com will be handled by /
othismo image sym-link /controller /server.sites/othismo.com 
```