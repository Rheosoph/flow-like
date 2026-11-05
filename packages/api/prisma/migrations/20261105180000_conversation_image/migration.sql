-- A group's picture: the blake3 hex of the PNG stored at `conversations/<id>/image/<image>.png`.
ALTER TABLE "Conversation" ADD COLUMN "image" TEXT;
