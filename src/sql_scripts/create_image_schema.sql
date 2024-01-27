create table module
(
    module_key INTEGER PRIMARY KEY,
    path       TEXT not null,
    wasm       BLOB not null UNIQUE
);