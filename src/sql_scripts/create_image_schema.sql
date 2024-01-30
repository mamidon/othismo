create table namespace
(
    path            TEXT PRIMARY KEY,
    object_key      INTEGER not null,
    FOREIGN KEY (object_key) REFERENCES object(object_key)
);

create table object
(
    object_key  INTEGER PRIMARY KEY,
    kind        TEXT CHECK ( kind IN ('MODULE', 'INSTANCE') ) not null,
    bytes       BLOB not null,
);

create table link
(
    link_key    INTEGER PRIMARY KEY,
    from_object_key     INTEGER not null,
    to_object_key       INTEGER not null,
    kind                TEXT CHECK ( kind IN ('INSTANCE_OF') ),
    FOREIGN KEY (from_object_key) REFERENCES object(object_key),
    FOREIGN KEY (to_object_key) REFERENCES  object(object_key)
)