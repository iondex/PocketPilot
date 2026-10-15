-- In private chats the bot account is part of the native conversation address.
ALTER TABLE containers RENAME TO old_containers;
CREATE TABLE containers (
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    instance_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    PRIMARY KEY(bot_id, chat_id, topic_id)
);
INSERT INTO containers(bot_id,chat_id,topic_id,instance_id,thread_id)
SELECT b.bot_id,c.chat_id,c.topic_id,c.instance_id,c.thread_id
FROM old_containers c JOIN bindings b ON b.chat_id=c.chat_id AND b.topic_id=c.topic_id;
DROP TABLE old_containers;
