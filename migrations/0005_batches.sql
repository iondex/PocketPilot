ALTER TABLE deliveries ADD COLUMN batch_opened_ms INTEGER;
ALTER TABLE deliveries ADD COLUMN batch_members INTEGER NOT NULL DEFAULT 0;
