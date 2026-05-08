/*
SQLite doesn't support UNSIGNED BIGINT
so seqs can *theoretically* overflow, but a signed 64-bit integer
is large enough to where it realistically won't be an issue, and by the time
it is, using SQLite will likely be a bigger problem itself
*/

CREATE TABLE UsedPublicKey (
    public_key TEXT PRIMARY KEY
);

CREATE TABLE LogEntry (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    kind INTEGER NOT NULL
);

CREATE TABLE RegisterIdentityLogEntry (
    seq INTEGER PRIMARY KEY,
    sig TEXT NOT NULL,

    identity_id TEXT NOT NULL UNIQUE,
    public_key TEXT NOT NULL UNIQUE,

    FOREIGN KEY (seq) REFERENCES LogEntry (seq),
    FOREIGN KEY (public_key) REFERENCES UsedPublicKey (public_key)
);

CREATE TABLE IdentityRotationLogEntry (
    seq INTEGER PRIMARY KEY,
    sig TEXT NOT NULL,

    identity_id TEXT NOT NULL,
    prev_rotation INTEGER UNIQUE,
    new_public_key TEXT NOT NULL,

    FOREIGN KEY (seq) REFERENCES LogEntry (seq),
    FOREIGN KEY (identity_id) REFERENCES RegisterIdentityLogEntry (identity_id),
    FOREIGN KEY (prev_rotation) REFERENCES IdentityRotationLogEntry (seq),
    FOREIGN KEY (new_public_key) REFERENCES UsedPublicKey (public_key)
);

CREATE INDEX idx_rotation_identity ON IdentityRotationLogEntry (identity_id);
CREATE UNIQUE INDEX idx_one_root_rotation_per_identity ON IdentityRotationLogEntry (identity_id)
    WHERE prev_rotation IS NULL;

CREATE TABLE ScopeManifest (
    scope TEXT NOT NULL,
    seq INTEGER PRIMARY KEY,

    owner TEXT NOT NULL,

    FOREIGN KEY (seq) REFERENCES LogEntry (seq),
    FOREIGN KEY (owner) REFERENCES RegisterIdentityLogEntry (identity_id)
);

CREATE INDEX idx_scope_manifest_scope_seq ON ScopeManifest (scope, seq DESC);
CREATE INDEX idx_scope_manifest_owner ON ScopeManifest (owner);

CREATE TABLE ScopeManifestMember (
    scope TEXT NOT NULL,
    seq INTEGER NOT NULL,

    identity_id TEXT NOT NULL,
    permissions TEXT NOT NULL,

    PRIMARY KEY (scope, seq, identity_id),
    FOREIGN KEY (scope, seq) REFERENCES ScopeManifest (scope, seq),
    FOREIGN KEY (identity_id) REFERENCES RegisterIdentityLogEntry (identity_id)
);

CREATE INDEX idx_manifest_member_identity ON ScopeManifestMember (identity_id);

CREATE TABLE ScopeLogEntry (
    seq INTEGER PRIMARY KEY,
    sig TEXT NOT NULL,

    scope TEXT NOT NULL,

    prev_scope_entry_hash TEXT,
    scope_seq INTEGER NOT NULL,
    prev_author_identity_seq INTEGER,
    author_identity TEXT NOT NULL,

    kind INTEGER NOT NULL,

    UNIQUE (scope, scope_seq),
    FOREIGN KEY (seq) REFERENCES LogEntry (seq),
    FOREIGN KEY (prev_author_identity_seq) REFERENCES IdentityRotationLogEntry (seq),
    FOREIGN KEY (author_identity) REFERENCES RegisterIdentityLogEntry (identity_id)
);

CREATE INDEX idx_scope_entry_author ON ScopeLogEntry (author_identity);
CREATE UNIQUE INDEX idx_one_root_entry_per_scope ON ScopeLogEntry (scope)
    WHERE prev_scope_entry_hash IS NULL;

CREATE TABLE PublishScopeLogEntry (
    scope TEXT NOT NULL,
    scope_seq INTEGER NOT NULL,

    name TEXT NOT NULL,
    version TEXT NOT NULL,
    archive_hash TEXT NOT NULL,

    PRIMARY KEY (scope, scope_seq),
    FOREIGN KEY (scope, scope_seq) REFERENCES ScopeLogEntry (scope, scope_seq)
);

CREATE INDEX idx_publish_name ON PublishScopeLogEntry (scope, name);

CREATE TABLE YankScopeLogEntry (
    scope TEXT NOT NULL,
    scope_seq INTEGER NOT NULL,

    name TEXT NOT NULL,
    version TEXT NOT NULL,

    PRIMARY KEY (scope, scope_seq),
    FOREIGN KEY (scope, scope_seq) REFERENCES ScopeLogEntry (scope, scope_seq)
);

CREATE INDEX idx_yank_name ON YankScopeLogEntry (scope, name);

CREATE TABLE DeprecateScopeLogEntry (
    scope TEXT NOT NULL,
    scope_seq INTEGER NOT NULL,

    name TEXT NOT NULL,
    reason TEXT NOT NULL,

    PRIMARY KEY (scope, scope_seq),
    FOREIGN KEY (scope, scope_seq) REFERENCES ScopeLogEntry (scope, scope_seq)
);

CREATE INDEX idx_deprecate_name ON DeprecateScopeLogEntry (scope, name);