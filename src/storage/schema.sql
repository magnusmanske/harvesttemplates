-- Applied at startup; every statement must be idempotent.

CREATE TABLE IF NOT EXISTS run (
    id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY,
    user_id BIGINT UNSIGNED NOT NULL,
    user_name VARCHAR(255) NOT NULL,
    share_id BIGINT UNSIGNED NULL,
    spec MEDIUMTEXT NOT NULL,
    status VARCHAR(16) NOT NULL,
    editgroup CHAR(12) NOT NULL,
    excluded TEXT NULL,
    message TEXT NULL,
    created BIGINT NOT NULL,
    started BIGINT NULL,
    finished BIGINT NULL,
    KEY user_runs (user_id, id),
    KEY status (status)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_bin;

CREATE TABLE IF NOT EXISTS run_row (
    run_id BIGINT UNSIGNED NOT NULL,
    seq INT UNSIGNED NOT NULL,
    page_id INT UNSIGNED NOT NULL,
    title VARCHAR(512) NOT NULL,
    item VARCHAR(16) NULL,
    status VARCHAR(10) NOT NULL,
    raw_value TEXT NULL,
    value TEXT NULL,
    message TEXT NULL,
    PRIMARY KEY (run_id, seq),
    KEY run_status (run_id, status, seq)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_bin;

CREATE TABLE IF NOT EXISTS share (
    id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY,
    user_id BIGINT UNSIGNED NOT NULL,
    user_name VARCHAR(255) NOT NULL,
    title VARCHAR(255) NOT NULL,
    spec MEDIUMTEXT NOT NULL,
    created BIGINT NOT NULL,
    last_run_id BIGINT UNSIGNED NULL,
    last_completed BIGINT NULL,
    last_done INT UNSIGNED NULL,
    last_errors INT UNSIGNED NULL
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_bin;

CREATE TABLE IF NOT EXISTS share_tag (
    share_id BIGINT UNSIGNED NOT NULL,
    tag VARCHAR(64) NOT NULL,
    PRIMARY KEY (share_id, tag),
    KEY tag (tag)
) DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_bin;
