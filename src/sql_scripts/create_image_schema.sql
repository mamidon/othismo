create table module
(
    module_key INTEGER PRIMARY KEY,
    wasm       BLOB not null
);

create table instance
(
    instance_key    INTEGER PRIMARY KEY,
    module_key      INTEGER,
    memory          BLOB not null,
    FOREIGN KEY (module_key) REFERENCES module(module_key)
);

create table namespace
(
    path            TEXT PRIMARY KEY,
    module_key      INTEGER,
    instance_key    INTEGER,
    FOREIGN KEY (module_key) REFERENCES module(module_key),
    FOREIGN KEY (instance_key) REFERENCES instance(instance_key)
);