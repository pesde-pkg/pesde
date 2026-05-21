CREATE TABLE UsedPublicKey (
    id BIGINT UNSIGNED PRIMARY KEY AUTO_INCREMENT,
    algorithm ENUM ('ssh-ed25519') NOT NULL,
    public_key VARBINARY(1024) NOT NULL UNIQUE
);

CREATE TABLE LogEntry (
    seq BIGINT UNSIGNED PRIMARY KEY,
    kind ENUM ('register_identity', 'identity_rotation', 'scope', 'admin_scope_transfer') NOT NULL
);

CREATE TABLE RegisterIdentityLogEntry (
    seq BIGINT UNSIGNED PRIMARY KEY,
    sig TEXT NOT NULL,

    identity_id BINARY(16) NOT NULL UNIQUE,
    public_key_id BIGINT UNSIGNED NOT NULL UNIQUE,

    FOREIGN KEY (seq) REFERENCES LogEntry (seq),
    FOREIGN KEY (public_key_id) REFERENCES UsedPublicKey (id)
);

CREATE TABLE IdentityRotationLogEntry (
    seq BIGINT UNSIGNED PRIMARY KEY,
    sig TEXT NOT NULL,

    identity_id BINARY(16) NOT NULL,
    prev_rotation BIGINT UNSIGNED UNIQUE,
    new_public_key_id BIGINT UNSIGNED NOT NULL,

    root_identity_id BINARY(16) AS (IF(prev_rotation IS NULL, identity_id, NULL)) STORED,

    FOREIGN KEY (seq) REFERENCES LogEntry (seq),
    FOREIGN KEY (identity_id) REFERENCES RegisterIdentityLogEntry (identity_id),
    FOREIGN KEY (prev_rotation) REFERENCES IdentityRotationLogEntry (seq),
    FOREIGN KEY (new_public_key_id) REFERENCES UsedPublicKey (id)
);

CREATE INDEX idx_rotation_identity ON IdentityRotationLogEntry (identity_id);
CREATE UNIQUE INDEX idx_one_root_rotation_per_identity ON IdentityRotationLogEntry (root_identity_id);

CREATE TABLE ScopeManifest (
    scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    seq BIGINT UNSIGNED PRIMARY KEY,

    owner BINARY(16) NOT NULL,

    FOREIGN KEY (seq) REFERENCES LogEntry (seq),
    FOREIGN KEY (owner) REFERENCES RegisterIdentityLogEntry (identity_id)
);

CREATE INDEX idx_scope_manifest_scope_seq ON ScopeManifest (scope, seq DESC);
CREATE INDEX idx_scope_manifest_owner ON ScopeManifest (owner);

CREATE TABLE ScopeManifestMember (
    scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    seq BIGINT UNSIGNED NOT NULL,

    identity_id BINARY(16) NOT NULL,
    permissions SET ('publish', 'retire') NOT NULL,

    PRIMARY KEY (scope, seq, identity_id),
    FOREIGN KEY (scope, seq) REFERENCES ScopeManifest (scope, seq),
    FOREIGN KEY (identity_id) REFERENCES RegisterIdentityLogEntry (identity_id)
);

CREATE INDEX idx_manifest_member_identity ON ScopeManifestMember (identity_id);

CREATE TABLE ScopeLogEntry (
    seq BIGINT UNSIGNED PRIMARY KEY,
    sig TEXT NOT NULL,

    scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,

    prev_scope_entry_hash TEXT,
    scope_seq BIGINT UNSIGNED NOT NULL,
    prev_author_identity_seq BIGINT UNSIGNED,
    author_identity BINARY(16) NOT NULL,

    kind ENUM ('publish', 'yank', 'deprecate', 'manifest_update') NOT NULL,

    root_scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin AS (IF(prev_scope_entry_hash IS NULL, scope, NULL)) STORED,

    UNIQUE (scope, scope_seq),
    FOREIGN KEY (seq) REFERENCES LogEntry (seq),
    FOREIGN KEY (prev_author_identity_seq) REFERENCES IdentityRotationLogEntry (seq),
    FOREIGN KEY (author_identity) REFERENCES RegisterIdentityLogEntry (identity_id)
);

CREATE INDEX idx_scope_entry_author ON ScopeLogEntry (author_identity);
CREATE UNIQUE INDEX idx_one_root_entry_per_scope ON ScopeLogEntry (root_scope);

CREATE TABLE PublishScopeLogEntry (
    scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    scope_seq BIGINT UNSIGNED NOT NULL,

    name VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    version VARCHAR(255) NOT NULL,
    archive_hash TEXT NOT NULL,

    PRIMARY KEY (scope, scope_seq),
    FOREIGN KEY (scope, scope_seq) REFERENCES ScopeLogEntry (scope, scope_seq)
);

CREATE INDEX idx_publish_name ON PublishScopeLogEntry (scope, name);

CREATE TABLE YankScopeLogEntry (
    scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    scope_seq BIGINT UNSIGNED NOT NULL,

    name VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    version VARCHAR(255) NOT NULL,

    PRIMARY KEY (scope, scope_seq),
    FOREIGN KEY (scope, scope_seq) REFERENCES ScopeLogEntry (scope, scope_seq)
);

CREATE INDEX idx_yank_package ON YankScopeLogEntry (scope, name, version);

CREATE TABLE DeprecateScopeLogEntry (
    scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    scope_seq BIGINT UNSIGNED NOT NULL,

    name VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    reason VARCHAR(255) NOT NULL,

    PRIMARY KEY (scope, scope_seq),
    FOREIGN KEY (scope, scope_seq) REFERENCES ScopeLogEntry (scope, scope_seq)
);

CREATE INDEX idx_deprecate_package ON DeprecateScopeLogEntry (scope, name);