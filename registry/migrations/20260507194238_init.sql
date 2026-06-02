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
    pos BIGINT UNSIGNED PRIMARY KEY,
    kind ENUM ('register_identity', 'identity_rotation', 'scope', 'admin_scope_transfer') NOT NULL,
    FOREIGN KEY (pos) REFERENCES TreeNode (pos)
);

CREATE TABLE RegisterIdentityLogEntry (
    pos BIGINT UNSIGNED PRIMARY KEY,
    sig TEXT NOT NULL,

    identity_id BINARY(16) NOT NULL UNIQUE,
    public_key_id BIGINT UNSIGNED NOT NULL UNIQUE,

    FOREIGN KEY (pos) REFERENCES LogEntry (pos),
    FOREIGN KEY (public_key_id) REFERENCES UsedPublicKey (id)
);

CREATE TABLE IdentityRotationLogEntry (
    pos BIGINT UNSIGNED PRIMARY KEY,
    sig TEXT NOT NULL,

    identity_id BINARY(16) NOT NULL,
    new_public_key_id BIGINT UNSIGNED NOT NULL,

    FOREIGN KEY (pos) REFERENCES LogEntry (pos),
    FOREIGN KEY (identity_id) REFERENCES RegisterIdentityLogEntry (identity_id),
    FOREIGN KEY (new_public_key_id) REFERENCES UsedPublicKey (id)
);

CREATE INDEX idx_rotation_identity ON IdentityRotationLogEntry (identity_id);

CREATE TABLE ScopeManifest (
    pos BIGINT UNSIGNED PRIMARY KEY,
    scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,

    owner BINARY(16) NOT NULL,

    FOREIGN KEY (pos) REFERENCES LogEntry (pos),
    FOREIGN KEY (owner) REFERENCES RegisterIdentityLogEntry (identity_id)
);

CREATE TABLE ScopeManifestMember (
    pos BIGINT UNSIGNED NOT NULL,

    identity_id BINARY(16) NOT NULL,
    permissions BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (pos, identity_id),
    FOREIGN KEY (pos) REFERENCES ScopeManifest (pos),
    FOREIGN KEY (identity_id) REFERENCES RegisterIdentityLogEntry (identity_id)
);

CREATE INDEX idx_manifest_member_identity ON ScopeManifestMember (identity_id);

CREATE TABLE ScopeLogEntry (
    pos BIGINT UNSIGNED PRIMARY KEY,
    sig TEXT NOT NULL,

    scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,

    author_identity BINARY(16) NOT NULL,

    kind ENUM ('publish', 'yank', 'deprecate', 'manifest_update') NOT NULL,

    FOREIGN KEY (pos) REFERENCES LogEntry (pos),
    FOREIGN KEY (author_identity) REFERENCES RegisterIdentityLogEntry (identity_id)
);

CREATE INDEX idx_scope ON ScopeLogEntry (scope);

CREATE TABLE PublishScopeLogEntry (
    pos BIGINT UNSIGNED PRIMARY KEY,

    name VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    version VARCHAR(255) NOT NULL,
    archive_hash TEXT NOT NULL,

    FOREIGN KEY (pos) REFERENCES ScopeLogEntry (pos)
);

CREATE INDEX idx_publish_package ON PublishScopeLogEntry (name, version);

CREATE TABLE YankScopeLogEntry (
    pos BIGINT UNSIGNED PRIMARY KEY,

    publish_pos BIGINT UNSIGNED NOT NULL UNIQUE,

    FOREIGN KEY (pos) REFERENCES ScopeLogEntry (pos),
    FOREIGN KEY (publish_pos) REFERENCES PublishScopeLogEntry (pos)
);

CREATE TABLE DeprecateScopeLogEntry (
    pos BIGINT UNSIGNED PRIMARY KEY,

    name VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    reason VARCHAR(255) NOT NULL,

    FOREIGN KEY (pos) REFERENCES ScopeLogEntry (pos)
);

CREATE INDEX idx_deprecate_package ON DeprecateScopeLogEntry (name);