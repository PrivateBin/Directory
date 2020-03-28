CREATE TABLE instances (
    id INTEGER NOT NULL PRIMARY KEY,
    url VARCHAR(255) NOT NULL,
    version VARCHAR(255) NOT NULL,
    https BOOLEAN NOT NULL DEFAULT 0,
    https_redirect BOOLEAN NOT NULL DEFAULT 0,
    country_id CHARACTER(2) NOT NULL DEFAULT "AQ"
);