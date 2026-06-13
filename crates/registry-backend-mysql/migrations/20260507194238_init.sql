CREATE TABLE Tree (
    _id BIT(1) PRIMARY KEY DEFAULT 0 CHECK (_id = 0),
    size BIGINT UNSIGNED NOT NULL
);

INSERT INTO Tree (size) VALUES (0);

CREATE TABLE TreeNode (
    pos BIGINT UNSIGNED PRIMARY KEY,
    sha512 BINARY(64) NOT NULL    
);

CREATE TABLE LogEntry (
    pos BIGINT UNSIGNED PRIMARY KEY,
    published_at DATETIME NOT NULL DEFAULT(NOW()),
    kind ENUM ('register_identity', 'identity_rotation', 'scope', 'admin_scope_transfer') NOT NULL,
    FOREIGN KEY (pos) REFERENCES TreeNode (pos)
);

CREATE TABLE Identity (
    identity_id BINARY(16) PRIMARY KEY
);

CREATE TABLE IdentityKeyEntry (
    pos BIGINT UNSIGNED PRIMARY KEY,
    sig TEXT NOT NULL,
    authorising_sig TEXT,

    identity_id BINARY(16) NOT NULL,
    algorithm ENUM ('ed25519') NOT NULL,
    public_key VARBINARY(1024) NOT NULL,

    UNIQUE (algorithm, public_key),
    FOREIGN KEY (pos) REFERENCES LogEntry (pos),
    FOREIGN KEY (identity_id) REFERENCES Identity (identity_id)
);

CREATE INDEX idx_identity_key_entry_identity ON IdentityKeyEntry (identity_id);

CREATE TABLE Scope (
    genesis_pos BIGINT UNSIGNED PRIMARY KEY,
    scope VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL UNIQUE,
    FOREIGN KEY (genesis_pos) REFERENCES LogEntry (pos)
);

CREATE TABLE ScopeManifest (
    pos BIGINT UNSIGNED PRIMARY KEY,
    scope_pos BIGINT UNSIGNED NOT NULL,

    owner BINARY(16) NOT NULL,

    FOREIGN KEY (pos) REFERENCES LogEntry (pos),
    FOREIGN KEY (scope_pos) REFERENCES Scope (genesis_pos),
    FOREIGN KEY (owner) REFERENCES Identity (identity_id)
);

CREATE TABLE ScopeManifestMember (
    pos BIGINT UNSIGNED NOT NULL,

    identity_id BINARY(16) NOT NULL,
    -- a specific package name granting write access to it, or '' for all packages
    package VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL DEFAULT '',

    PRIMARY KEY (pos, identity_id, package),
    FOREIGN KEY (pos) REFERENCES ScopeManifest (pos),
    FOREIGN KEY (identity_id) REFERENCES Identity (identity_id)
);

CREATE INDEX idx_manifest_member_identity ON ScopeManifestMember (identity_id);

CREATE TABLE ScopeLogEntry (
    pos BIGINT UNSIGNED PRIMARY KEY,
    sig TEXT NOT NULL,

    scope_pos BIGINT UNSIGNED NOT NULL,

    author_identity BINARY(16) NOT NULL,

    kind ENUM ('publish', 'yank', 'deprecate', 'manifest_update') NOT NULL,

    FOREIGN KEY (pos) REFERENCES LogEntry (pos),
    FOREIGN KEY (scope_pos) REFERENCES Scope (genesis_pos),
    FOREIGN KEY (author_identity) REFERENCES Identity (identity_id)
);

CREATE TABLE Package (
    genesis_pos BIGINT UNSIGNED PRIMARY KEY,
    scope_pos BIGINT UNSIGNED NOT NULL,
    name VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    UNIQUE (scope_pos, name),
    FOREIGN KEY (genesis_pos) REFERENCES ScopeLogEntry (pos),
    FOREIGN KEY (scope_pos) REFERENCES Scope (genesis_pos)
);

CREATE TABLE PublishScopeLogEntry (
    pos BIGINT UNSIGNED PRIMARY KEY,

    package_pos BIGINT UNSIGNED NOT NULL,
    version VARCHAR(255) CHARACTER SET ascii COLLATE ascii_general_ci NOT NULL,
    version_ord VARBINARY(512) NOT NULL,
    archive_hash TEXT NOT NULL,

    description VARCHAR(255) NOT NULL,
    license VARCHAR(255) NOT NULL,
    repository TEXT NOT NULL,

    UNIQUE (package_pos, version),
    FOREIGN KEY (pos) REFERENCES ScopeLogEntry (pos),
    FOREIGN KEY (package_pos) REFERENCES Package (genesis_pos)
);

CREATE INDEX idx_publish_version_seq ON PublishScopeLogEntry (version_ord);

CREATE TABLE PublishAuthor (
    pos BIGINT UNSIGNED NOT NULL,
    seq TINYINT UNSIGNED NOT NULL,
    author VARCHAR(255) NOT NULL,

    PRIMARY KEY (pos, seq),
    FOREIGN KEY (pos) REFERENCES PublishScopeLogEntry (pos)
);

CREATE TABLE PublishDependency (
    pos BIGINT UNSIGNED NOT NULL,
    alias VARCHAR(255) CHARACTER SET ascii NOT NULL,
    dependency_type ENUM ('standard', 'peer', 'dev') NOT NULL,

    kind ENUM ('pesde', 'wally') NOT NULL,
    name VARCHAR(255) CHARACTER SET ascii NOT NULL,
    version_req VARCHAR(255) CHARACTER SET ascii NOT NULL,
    registry TEXT,
    realm ENUM ('shared', 'server'),

    PRIMARY KEY (pos, alias),
    FOREIGN KEY (pos) REFERENCES PublishScopeLogEntry (pos)
);

CREATE TABLE YankScopeLogEntry (
    pos BIGINT UNSIGNED PRIMARY KEY,

    publish_pos BIGINT UNSIGNED NOT NULL,
    action ENUM ('add', 'revoke') NOT NULL,

    FOREIGN KEY (pos) REFERENCES ScopeLogEntry (pos),
    FOREIGN KEY (publish_pos) REFERENCES PublishScopeLogEntry (pos)
);

CREATE TABLE DeprecateScopeLogEntry (
    pos BIGINT UNSIGNED PRIMARY KEY,

    package_pos BIGINT UNSIGNED NOT NULL,
    reason VARCHAR(255) NOT NULL,

    FOREIGN KEY (pos) REFERENCES ScopeLogEntry (pos),
    FOREIGN KEY (package_pos) REFERENCES Package (genesis_pos)
);
