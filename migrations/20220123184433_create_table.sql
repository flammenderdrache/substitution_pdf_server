-- Add migration script here

CREATE TABLE substitution_json
(
    hash           TEXT UNIQUE ,
    pdf_date       TIMESTAMP NOT NULL,
    insertion_time TIMESTAMP,
    json           jsonb
);
