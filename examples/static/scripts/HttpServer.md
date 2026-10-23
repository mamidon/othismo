# HttpServer

HttpServer turns HTTP requests into requests of its view, and their answers
into responses. It knows nothing about files: what it serves is whatever the
image mounts where its hosts point.

## Hosts

Each host name it answers for has a root, a path in its view, which only
whoever runs the image sets, by writing it. With `http` mounted at `/http`
in the image's view:

```text
write /http/hosts/* "/www"
write /http/hosts/files.localhost "/www/static"
```

`*` stands for any host not listed. Removing a host stops serving it.
`list /hosts` lists them, and `read /hosts/<name>` answers its root.

## Requests

The `Host` header, lowercased and without its port, picks the root, and the
URL's path, percent-decoded, is added to it. A host it doesn't serve gets a
404, and a path that would climb out of the root a 400.

| Method | Request |
|---|---|
| GET | `read` |
| HEAD | `stat` |
| PUT, DELETE, POST | none: 405, allowing GET and HEAD |
| anything else | none: 501 |

## Responses

| Answer | Response |
|---|---|
| a payload | 200, with its content type, or `application/octet-stream` if it has none |
| a stat record | 200, with its content type and length and no body, for HEAD |
| no payload | 204 |
| a directory: `is_directory`, or a stat record of one | 301 to the path with a `/` added, or 404 if it already has one |
| `not_found` | 404 |
| `read_only`, `not_supported` | 405 |
| `forbidden` | 403 |
| `invalid` | 400 |
| any other failure | 500 |

A malformed request gets a 400, a head over 16 KiB a 431, and a body over
1 MiB a 413. There is one request per connection: every response closes it,
and is logged to `/dev/cons` as `<method> <path> <status>`.
