CREATE TABLE UsedPublicKey (
    id BIGINT UNSIGNED PRIMARY KEY AUTO_INCREMENT,
    algorithm ENUM ('ssh-ed25519') NOT NULL,
    public_key VARBINARY(1024) NOT NULL,
    UNIQUE (algorithm, public_key)
);

CREATE TABLE TreeNode (
    pos BIGINT UNSIGNED PRIMARY KEY,
    sha256 BINARY(32) NOT NULL    
);

CREATE TABLE LogEntry (
    seq BIGINT UNSIGNED PRIMARY KEY,
    pos BIGINT UNSIGNED NOT NULL,
    kind ENUM ('register_identity', 'identity_rotation', 'scope', 'admin_scope_transfer') NOT NULL,
    FOREIGN KEY (pos) REFERENCES TreeNode (pos)
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
    new_public_key_id BIGINT UNSIGNED NOT NULL,

    FOREIGN KEY (seq) REFERENCES LogEntry (seq),
    FOREIGN KEY (identity_id) REFERENCES RegisterIdentityLogEntry (identity_id),
    FOREIGN KEY (new_public_key_id) REFERENCES UsedPublicKey (id)
);

CREATE INDEX idx_rotation_identity ON IdentityRotationLogEntry (identity_id);

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
    permissions BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (scope, seq, identity_id),
    FOREIGN KEY (scope, seq) REFERENCES ScopeManifest (scope, seq),
    FOREIGN KEY (identity_id) REFERENCES RegisterIdentityLogEntry (identity_id)
);

CREATE INDEX idx_manifest_member_identity ON ScopeManifestMember (identity_id);

CREATE TABLE ScopeLogEntry (
    seq BIGINT UNSIGNED PRIMARY KEY,
    sig TEXT NOT NULL,

    scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,

    scope_seq BIGINT UNSIGNED NOT NULL,
    author_identity BINARY(16) NOT NULL,

    kind ENUM ('publish', 'yank', 'deprecate', 'manifest_update') NOT NULL,

    UNIQUE (scope, scope_seq),
    FOREIGN KEY (seq) REFERENCES LogEntry (seq),
    FOREIGN KEY (author_identity) REFERENCES RegisterIdentityLogEntry (identity_id)
);

CREATE INDEX idx_scope_entry_author ON ScopeLogEntry (author_identity);

CREATE TABLE PublishScopeLogEntry (
    scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    scope_seq BIGINT UNSIGNED NOT NULL,

    name VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    version VARCHAR(255) NOT NULL,
    archive_hash TEXT NOT NULL,

    PRIMARY KEY (scope, scope_seq),
    FOREIGN KEY (scope, scope_seq) REFERENCES ScopeLogEntry (scope, scope_seq)
);

CREATE INDEX idx_publish_package ON PublishScopeLogEntry (scope, name, version);

CREATE TABLE YankScopeLogEntry (
    scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    scope_seq BIGINT UNSIGNED NOT NULL,

    name VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    version VARCHAR(255) NOT NULL,

    PRIMARY KEY (scope, scope_seq),
    FOREIGN KEY (scope, scope_seq) REFERENCES ScopeLogEntry (scope, scope_seq),
    FOREIGN KEY (scope, name, version) REFERENCES PublishScopeLogEntry (scope, name, version)
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