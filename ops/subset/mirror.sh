#!/bin/bash
FILE_ID=1ch6ZYOyq4BCOlspiXgqJV7y-CC50w8CO
S50=${S50:-/home/sandbox/swbench/subset50}
echo "$(date '+%F %T') mirror loop started" >> /tmp/swe-mirror.log
while true; do
  bash "$(dirname "$0")/mk-mirror.sh"
  tools google-drive update --file-id "$FILE_ID" --file-path /tmp/mirror.txt --mime-type text/plain >/dev/null 2>&1 \
    || echo "$(date '+%F %T') update failed" >> /tmp/swe-mirror.log
  if [ -f "$S50/.subset_complete" ]; then
    sleep 20; bash "$(dirname "$0")/mk-mirror.sh"
    tools google-drive update --file-id "$FILE_ID" --file-path /tmp/mirror.txt --mime-type text/plain >/dev/null 2>&1
    echo "$(date '+%F %T') complete, final update done" >> /tmp/swe-mirror.log
    exit 0
  fi
  sleep 45
done
