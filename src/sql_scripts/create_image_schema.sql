create table module
(
    module_key INT  not null
        constraint module_pk
            primary key,
    path       TEXT not null,
    wasm       BLOB not null UNIQUE
);