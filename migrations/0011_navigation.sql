CREATE TABLE navigation (
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    id TEXT NOT NULL UNIQUE,
    payload TEXT NOT NULL,
    after_message_id INTEGER NOT NULL,
    message_id INTEGER,
    published_at INTEGER,
    consumed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(bot_id,chat_id,topic_id)
);
