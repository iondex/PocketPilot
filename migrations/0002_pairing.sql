CREATE TABLE pairing_requests (
    code_hash TEXT PRIMARY KEY,
    bot_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL CHECK (user_id > 0),
    chat_id INTEGER NOT NULL CHECK (chat_id = user_id),
    expires_at INTEGER NOT NULL,
    UNIQUE(bot_id, user_id)
);

CREATE TABLE pairings (
    bot_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL CHECK (user_id > 0),
    chat_id INTEGER NOT NULL CHECK (chat_id = user_id),
    paired_at INTEGER NOT NULL,
    PRIMARY KEY(bot_id, user_id)
);
