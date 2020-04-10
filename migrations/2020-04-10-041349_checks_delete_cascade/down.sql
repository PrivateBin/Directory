ALTER TABLE checks RENAME TO _checks_new;

CREATE TABLE checks (
    id INTEGER NOT NULL PRIMARY KEY,
    updated TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    up BOOLEAN NOT NULL DEFAULT 0,
    instance_id INTEGER NOT NULL,
    FOREIGN KEY(instance_id) REFERENCES instances(id)
);

INSERT INTO checks
SELECT * FROM _checks_new;

DROP TABLE _checks_new;