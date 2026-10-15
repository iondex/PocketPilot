CREATE TABLE operations (
    id TEXT PRIMARY KEY,
    source_key TEXT NOT NULL UNIQUE,
    instance_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('received','prepared','dispatching','accepted','rejected','outcome_unknown')),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE containers (
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    instance_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    PRIMARY KEY (chat_id, topic_id)
);

CREATE TABLE bindings (
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    instance_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    active INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (bot_id, chat_id, topic_id),
    UNIQUE (bot_id, chat_id, instance_id, thread_id)
);

CREATE TABLE inbound_updates (
    bot_id INTEGER NOT NULL,
    polling_epoch INTEGER NOT NULL,
    update_id INTEGER NOT NULL,
    event_key TEXT NOT NULL,
    payload TEXT NOT NULL,
    received_at INTEGER NOT NULL,
    processed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (bot_id, polling_epoch, update_id),
    UNIQUE (bot_id, event_key)
);

CREATE TABLE polling_cursors (
    bot_id INTEGER PRIMARY KEY,
    polling_epoch INTEGER NOT NULL,
    next_offset INTEGER,
    generation_lower_bound INTEGER NOT NULL
);

CREATE TABLE deliveries (
    id TEXT PRIMARY KEY,
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    source_key TEXT NOT NULL,
    payload TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending','dispatching','delivered','outcome_unknown','rejected')),
    message_id INTEGER,
    UNIQUE (bot_id, chat_id, topic_id, source_key)
);
