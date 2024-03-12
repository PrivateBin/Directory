ALTER TABLE instances RENAME TO _instances;

CREATE TABLE instances (
    id INTEGER NOT NULL PRIMARY KEY,
    url VARCHAR(255) NOT NULL,
    version VARCHAR(255) NOT NULL,
    https BOOLEAN NOT NULL DEFAULT 0,
    https_redirect BOOLEAN NOT NULL DEFAULT 0,
    country_id CHARACTER(2) NOT NULL DEFAULT "AQ",
    attachments BOOLEAN NOT NULL DEFAULT 0,
    csp_header BOOLEAN NOT NULL DEFAULT 0
);

INSERT INTO instances (id, url, version, https, https_redirect, country_id, attachments, csp_header)
SELECT id, url, version, https, https_redirect, country_id, attachments, csp_header
FROM _instances;

DROP TABLE _instances;

-- recreating these is necessary to ensure the references point to the new parent
ALTER TABLE checks RENAME TO _checks;

CREATE TABLE checks (
    id INTEGER NOT NULL PRIMARY KEY,
    updated TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    up BOOLEAN NOT NULL DEFAULT 0,
    instance_id INTEGER NOT NULL,
    FOREIGN KEY(instance_id) REFERENCES instances(id) ON DELETE CASCADE
);

INSERT INTO checks
SELECT * FROM _checks;

DROP TABLE _checks;

ALTER TABLE scans RENAME TO _scans;

CREATE TABLE scans (
    id INTEGER NOT NULL PRIMARY KEY,
    scanner VARCHAR(255) NOT NULL,
    rating VARCHAR(255) NOT NULL DEFAULT "-",
    percent INTEGER NOT NULL DEFAULT 0,
    instance_id INTEGER NOT NULL,
    FOREIGN KEY(instance_id) REFERENCES instances(id) ON DELETE CASCADE
);

INSERT INTO scans
SELECT * FROM _scans;

DROP TABLE _scans;
