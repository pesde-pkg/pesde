-- this table is purely for storage optimisation - PQ keys are much longer than 32 byte ed25519
CREATE TABLE public_key (
    id BIGINT UNSIGNED PRIMARY KEY AUTO_INCREMENT,
    algorithm ENUM ('ed25519') NOT NULL,
    data VARBINARY(4096) NOT NULL,

    UNIQUE (algorithm, data)
);

CREATE TABLE global_log (
    _id BIT(1) PRIMARY KEY DEFAULT 0 CHECK (_id = 0),
    size BIGINT UNSIGNED NOT NULL
);

INSERT INTO global_log (size) VALUES (0);

CREATE TABLE global_log_node (
    pos BIGINT UNSIGNED PRIMARY KEY,
    blake3 BINARY(32) NOT NULL
);

CREATE TABLE global_log_entry (
    pos BIGINT UNSIGNED PRIMARY KEY,
    published_at DATETIME NOT NULL DEFAULT(NOW()),

    FOREIGN KEY (pos) REFERENCES global_log_node (pos)
);

CREATE TABLE global_log_genesis_entry (
    pos BIGINT UNSIGNED PRIMARY KEY,
    scope BIGINT UNSIGNED NOT NULL UNIQUE,
    owner_id BIGINT UNSIGNED NOT NULL,

    FOREIGN KEY (pos) REFERENCES global_log_entry (pos),
    FOREIGN KEY (owner_id) REFERENCES public_key (id)
);

CREATE TABLE scope (
    genesis_pos BIGINT UNSIGNED PRIMARY KEY,
    name VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL UNIQUE,
    log_size BIGINT UNSIGNED NOT NULL,

    FOREIGN KEY (genesis_pos) REFERENCES global_log_genesis_entry (pos)
);

CREATE TABLE scope_log_node (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,
    blake3 BINARY(32) NOT NULL,

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope) REFERENCES scope (genesis_pos)
);

CREATE TABLE scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,
    
    published_at DATETIME NOT NULL DEFAULT(NOW()),
    
    sig BLOB,
    prev_blake3 BINARY(32) NOT NULL,
    versions_root_blake3 BINARY(32) NOT NULL,
    deprecated_root_blake3 BINARY(32) NOT NULL,

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES scope_log_node (scope, pos)
);

CREATE TABLE signed_scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,
    op_kind ENUM (
        'add_member',
        'update_member_grant',
        'rotate_key',
        'remove_member',
        'transfer_ownership',
        'publish_version',
        'set_yanked',
        'set_deprecation'
    ),
    signer BIGINT UNSIGNED,

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES scope_log_entry (scope, pos),
    FOREIGN KEY (signer) REFERENCES public_key (id)
);

CREATE TABLE scope_member_grant (
    id BIGINT UNSIGNED PRIMARY KEY AUTO_INCREMENT
);

CREATE TABLE scope_member_grant_package (
    grant_id BIGINT UNSIGNED NOT NULL,
    package VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,

    PRIMARY KEY (grant_id, package),
    FOREIGN KEY (grant_id) REFERENCES scope_member_grant (id)
);

CREATE TABLE scope_members_node (
    blake3 BINARY(32) PRIMARY KEY,
    kind ENUM ('leaf', 'internal') NOT NULL
);

CREATE TABLE scope_members_leaf_node_entry (
    node_blake3 BINARY(32) NOT NULL,
    idx SMALLINT UNSIGNED NOT NULL,

    member BIGINT UNSIGNED NOT NULL,
    grant_id BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (node_blake3, idx),
    FOREIGN KEY (node_blake3) REFERENCES scope_members_node (blake3),
    FOREIGN KEY (member) REFERENCES public_key (id),
    FOREIGN KEY (grant_id) REFERENCES scope_member_grant (id)
);

CREATE TABLE scope_members_internal_node_key (
    node_blake3 BINARY(32) NOT NULL,
    idx SMALLINT UNSIGNED NOT NULL,

    member BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (node_blake3, idx),
    FOREIGN KEY (node_blake3) REFERENCES scope_members_node (blake3),
    FOREIGN KEY (member) REFERENCES public_key (id)
);

CREATE TABLE scope_members_internal_node_child (
    node_blake3 BINARY(32) NOT NULL,
    idx SMALLINT UNSIGNED NOT NULL,
    
    child_blake3 BINARY(32) NOT NULL,

    PRIMARY KEY (node_blake3, idx),
    FOREIGN KEY (node_blake3) REFERENCES scope_members_node (blake3),
    FOREIGN KEY (child_blake3) REFERENCES scope_members_node (blake3)
);

CREATE TABLE signed_add_member_scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,  

    scope_members_root_blake3 BINARY(32) NOT NULL,

    member BIGINT UNSIGNED NOT NULL,
    grant_id BIGINT UNSIGNED NOT NULL,
    consent BLOB NOT NULL,
    nonce BINARY(16) NOT NULL UNIQUE,

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES signed_scope_log_entry (scope, pos),
    FOREIGN KEY (scope_members_root_blake3) REFERENCES scope_members_node (blake3),
    FOREIGN KEY (member) REFERENCES public_key (id),
    FOREIGN KEY (grant_id) REFERENCES scope_member_grant (id)
);

CREATE TABLE signed_update_member_grant_scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,  

    scope_members_root_blake3 BINARY(32) NOT NULL,

    member BIGINT UNSIGNED NOT NULL,
    grant_id BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES signed_scope_log_entry (scope, pos),
    FOREIGN KEY (scope_members_root_blake3) REFERENCES scope_members_node (blake3),
    FOREIGN KEY (member) REFERENCES public_key (id),
    FOREIGN KEY (grant_id) REFERENCES scope_member_grant (id)
);

CREATE TABLE signed_rotate_key_scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,  

    scope_members_root_blake3 BINARY(32) NOT NULL,

    new_member BIGINT UNSIGNED NOT NULL,
    proof BLOB NOT NULL,
    nonce BINARY(16) NOT NULL UNIQUE,

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES signed_scope_log_entry (scope, pos),
    FOREIGN KEY (scope_members_root_blake3) REFERENCES scope_members_node (blake3),
    FOREIGN KEY (new_member) REFERENCES public_key (id)
);

CREATE TABLE signed_remove_member_scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,  

    scope_members_root_blake3 BINARY(32) NOT NULL,

    member BIGINT UNSIGNED,

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES signed_scope_log_entry (scope, pos),
    FOREIGN KEY (scope_members_root_blake3) REFERENCES scope_members_node (blake3),
    FOREIGN KEY (member) REFERENCES public_key (id)
);

CREATE TABLE signed_transfer_ownership_scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,

    new_owner BIGINT UNSIGNED NOT NULL,
    proof BLOB NOT NULL,
    nonce BINARY(16) NOT NULL UNIQUE,    

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES signed_scope_log_entry (scope, pos),
    FOREIGN KEY (new_owner) REFERENCES public_key (id)
);

CREATE TABLE scope_package (
    scope BIGINT UNSIGNED NOT NULL,
    genesis_pos BIGINT UNSIGNED NOT NULL,
    
    name VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,

    PRIMARY KEY (scope, genesis_pos),
    FOREIGN KEY (scope) REFERENCES scope (genesis_pos),
    FOREIGN KEY (scope, genesis_pos) REFERENCES signed_scope_log_entry (scope, pos),
    UNIQUE (scope, name)
);

CREATE TABLE versions_node (
    blake3 BINARY(32) PRIMARY KEY,
    kind ENUM ('leaf', 'internal') NOT NULL
);

CREATE TABLE versions_leaf_node_entry (
    node_blake3 BINARY(32) NOT NULL,
    idx SMALLINT UNSIGNED NOT NULL,

    scope BIGINT UNSIGNED NOT NULL,
    version_pos BIGINT UNSIGNED NOT NULL,
    yank_state ENUM ('yanked', 'admin_yanked'),

    PRIMARY KEY (node_blake3, idx),
    FOREIGN KEY (node_blake3) REFERENCES versions_node (blake3),
    FOREIGN KEY (scope, version_pos) REFERENCES signed_scope_log_entry (scope, pos)
);

CREATE TABLE versions_internal_node_key (
    node_blake3 BINARY(32) NOT NULL,
    idx SMALLINT UNSIGNED NOT NULL,

    scope BIGINT UNSIGNED NOT NULL,
    version_pos BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (node_blake3, idx),
    FOREIGN KEY (node_blake3) REFERENCES versions_node (blake3),
    FOREIGN KEY (scope, version_pos) REFERENCES signed_scope_log_entry (scope, pos)
);

CREATE TABLE versions_internal_node_child (
    node_blake3 BINARY(32) NOT NULL,
    idx SMALLINT UNSIGNED NOT NULL,
    
    child_blake3 BINARY(32) NOT NULL,

    PRIMARY KEY (node_blake3, idx),
    FOREIGN KEY (node_blake3) REFERENCES versions_node (blake3),
    FOREIGN KEY (child_blake3) REFERENCES versions_node (blake3)
);

CREATE TABLE signed_publish_version_scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,

    versions_root_blake3 BINARY(32) NOT NULL,

    package_pos BIGINT UNSIGNED NOT NULL,
    version VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    archive_hash VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES signed_scope_log_entry (scope, pos),
    FOREIGN KEY (versions_root_blake3) REFERENCES versions_node (blake3),
    FOREIGN KEY (scope, package_pos) REFERENCES scope_package (scope, genesis_pos),
    UNIQUE (scope, package_pos, version)
);

CREATE TABLE signed_set_yanked_scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,  

    versions_root_blake3 BINARY(32) NOT NULL,

    package_version_pos BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES signed_scope_log_entry (scope, pos),
    FOREIGN KEY (versions_root_blake3) REFERENCES versions_node (blake3),
    FOREIGN KEY (scope, package_version_pos) REFERENCES signed_publish_version_scope_log_entry (scope, pos)
);

CREATE TABLE deprecations_node (
    blake3 BINARY(32) PRIMARY KEY,
    kind ENUM ('leaf', 'internal') NOT NULL
);

CREATE TABLE deprecations_leaf_node_entry (
    node_blake3 BINARY(32) NOT NULL,
    idx SMALLINT UNSIGNED NOT NULL,

    scope BIGINT UNSIGNED NOT NULL,
    package_pos BIGINT UNSIGNED NOT NULL,
    reason VARCHAR(255) NOT NULL,

    PRIMARY KEY (node_blake3, idx),
    FOREIGN KEY (node_blake3) REFERENCES deprecations_node (blake3),
    FOREIGN KEY (scope, package_pos) REFERENCES scope_package (scope, genesis_pos)
);

CREATE TABLE deprecations_internal_node_key (
    node_blake3 BINARY(32) NOT NULL,
    idx SMALLINT UNSIGNED NOT NULL,

    scope BIGINT UNSIGNED NOT NULL,
    package_pos BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (node_blake3, idx),
    FOREIGN KEY (node_blake3) REFERENCES deprecations_node (blake3),
    FOREIGN KEY (scope, package_pos) REFERENCES scope_package (scope, genesis_pos)
);

CREATE TABLE deprecations_internal_node_child (
    node_blake3 BINARY(32) NOT NULL,
    idx SMALLINT UNSIGNED NOT NULL,
    
    child_blake3 BINARY(32) NOT NULL,

    PRIMARY KEY (node_blake3, idx),
    FOREIGN KEY (node_blake3) REFERENCES deprecations_node (blake3),
    FOREIGN KEY (child_blake3) REFERENCES deprecations_node (blake3)
);

CREATE TABLE signed_set_deprecation_scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,  

    deprecated_root_blake3 BINARY(32) NOT NULL,

    package_pos BIGINT UNSIGNED NOT NULL,
    reason VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin,

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES signed_scope_log_entry (scope, pos),
    FOREIGN KEY (deprecated_root_blake3) REFERENCES deprecations_node (blake3),
    FOREIGN KEY (scope, package_pos) REFERENCES scope_package (scope, genesis_pos)
);

CREATE TABLE admin_scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,
    op_kind ENUM ('transfer_ownership', 'set_yanked'),

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES scope_log_entry (scope, pos)
);

CREATE TABLE admin_transfer_ownership_scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,
    
    new_owner BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES admin_scope_log_entry (scope, pos),
    FOREIGN KEY (new_owner) REFERENCES public_key (id)
);

CREATE TABLE admin_set_yanked_scope_log_entry (
    scope BIGINT UNSIGNED NOT NULL,
    pos BIGINT UNSIGNED NOT NULL,
    
    versions_root_blake3 BINARY(32) NOT NULL,

    package_version_pos BIGINT UNSIGNED NOT NULL,

    PRIMARY KEY (scope, pos),
    FOREIGN KEY (scope, pos) REFERENCES admin_scope_log_entry (scope, pos),
    FOREIGN KEY (versions_root_blake3) REFERENCES versions_node (blake3),
    FOREIGN KEY (scope, package_version_pos) REFERENCES signed_publish_version_scope_log_entry (scope, pos)
);
