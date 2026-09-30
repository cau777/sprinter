ALTER TABLE uploads ADD COLUMN text_chars INTEGER;
ALTER TABLE uploads ADD COLUMN text_pages INTEGER;
ALTER TABLE uploads ADD COLUMN text_empty_pages INTEGER;
ALTER TABLE uploads ADD COLUMN text_extractor TEXT;

CREATE TABLE message_attachments_new (
  message_id  TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  upload_id   TEXT NOT NULL REFERENCES uploads(id),
  position    INTEGER NOT NULL CHECK (position >= 0),
  PRIMARY KEY (message_id, upload_id)
);

INSERT INTO message_attachments_new (message_id, upload_id, position)
SELECT message_id, upload_id, position FROM message_attachments;

DROP TABLE message_attachments;
ALTER TABLE message_attachments_new RENAME TO message_attachments;
CREATE INDEX message_attachments_upload ON message_attachments(upload_id);

DELETE FROM settings WHERE key = 'pdf_engine';
