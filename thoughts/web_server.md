# Messaging Interface & PIDs

When configuring an image; we’re importing modules & instantiating instances.  Somehow we have to wire up these
Instances such that they can actually interact.

Option A: Direct configuration via messaging instances.  (Need to bind instances to names or IDs)
Option B: Massive manipulation of the namespace, ala Plan9.


### All Information Inside Messages

Instead of having multiple parameters, we encode everything inside the message via top level namespaces.  e.g.

```
{
  "othismo": {
    "send_to": "/namespace/foo",
    "reply_to": "/namespace/me" // optional
  },
  "acme.custom_message": {
    "foo": "bar"
  }
}
```


## Syscalls

### prepare_inbox(length: i32) -> (i32)
Given a minimum required length; the inbox buffer is 
resized if necessary.  Returns a pointer to an appropriate buffer.

The host must call this for every message it wishes to place in the inbox.

### message_received() -> ()
Triggers logic in the guest module to process whichever message has been placed into the inbox.

### send_message(length: i32, buffer: i32) -> (i32)
Tells the host that a message exists in the outbox.
The length of a response, placed into the inbox, is returned.

If no response is returned, the length is 0.

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

Messages with a single message can elide the `othismo` message; provided the top level
key takes a special form.  

Assuming you want to send a message `fizz.buzz` to object `/foo/foo.bar`.
```
{
    "/foo/foo.bar/fizz.buzz": {
        // data
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