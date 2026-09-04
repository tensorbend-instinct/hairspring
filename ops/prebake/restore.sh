#!/bin/bash
# Restore after box recycle: download chunks by ids in /tmp/prebake-ids.txt (mirrored to Drive mirror file tail),
# cat parts, untar. Replaces cargo build (~10min) + venv pip installs (~5min) with a download.
# ids file: re-fetch from the Drive mirror or /tmp/prebake-ids.txt if box survived.
set -eu
mkdir -p /tmp/restore && cd /tmp/restore
while read -r n id; do
  [ -n "$id" ] && tools google-drive download --file-id "$id" --json 2>/dev/null | jq -r '.file_path' | xargs -I{} mv {} "/tmp/restore/$n"
done < /tmp/prebake-ids.txt
cat target.part* > target.tar.gz; cat venvs.part* > venvs.tar.gz
mkdir -p /home/sandbox/hairspring /home/sandbox/swbench
tar xzf target.tar.gz -C /home/sandbox/hairspring
tar xzf venvs.tar.gz -C /home/sandbox/swbench
echo RESTORE_OK
