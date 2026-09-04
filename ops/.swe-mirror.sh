#!/bin/bash
FILE_ID=1ch6ZYOyq4BCOlspiXgqJV7y-CC50w8CO
echo "$(date '+%F %T') mirror loop started" >> /tmp/swe-mirror.log
while true; do
  bash /tmp/mk-mirror.sh
  tools google-drive update --file-id "$FILE_ID" --file-path /tmp/mirror.txt --mime-type text/plain >/dev/null 2>&1 \
    || echo "$(date '+%F %T') update failed" >> /tmp/swe-mirror.log
  if [ -f /home/sandbox/swbench/single/result.json ]; then
    sleep 20; bash /tmp/mk-mirror.sh
    tools google-drive update --file-id "$FILE_ID" --file-path /tmp/mirror.txt --mime-type text/plain >/dev/null 2>&1
    echo "$(date '+%F %T') result present, final update done" >> /tmp/swe-mirror.log
    exit 0
  fi
  [ -f /tmp/swe-mirror.stop ] && exit 0
  sleep 45
done
