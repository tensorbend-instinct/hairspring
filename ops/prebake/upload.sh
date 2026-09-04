#!/bin/bash
# Chunked pre-bake upload to Drive (25MB/file API limit). Detached: survives agent runs while box lives.
# Chunks + manifest land under Drive folder named by $FOLDER_ID (create once, id in manifest).
set -u
WORK=/tmp/prebake-chunks
mkdir -p "$WORK"
split -b 24m -d /tmp/prebake-target.tar.gz "$WORK/target.part" 2>/dev/null
split -b 24m -d /tmp/prebake-venvs.tar.gz "$WORK/venvs.part" 2>/dev/null
: > /tmp/prebake-ids.txt
for f in "$WORK"/*; do
  n=$(basename "$f")
  id=$(tools google-drive upload --file-path "$f" --name "hairspring-prebake-20260904-$n" --mime-type application/octet-stream --json 2>/dev/null | jq -r '.file.id // empty')
  echo "$n $id" >> /tmp/prebake-ids.txt
done
echo DONE > /tmp/prebake-upload.done
