#!/bin/sh
# Roda no emulador os executáveis de teste listados em $1 (um por linha), com as referências de
# crates/test/snapshots. Chamado pelo job `android` do CI.
set -eu

dir=/data/local/tmp/tomni-ui
adb shell rm -rf "$dir"
adb shell mkdir -p "$dir"
adb push crates/test/snapshots "$dir/snapshots" >/dev/null

status=0
while read -r exe; do
  name=$(basename "$exe")
  adb push "$exe" "$dir/$name" >/dev/null
  echo "== $name"
  adb shell "cd $dir && TOMNI_UI_SNAPSHOT_DIR=$dir/snapshots ./$name" || status=1
done < "$1"

# Diferenças, para o artefato do job.
adb pull "$dir/snapshots" android-snapshots >/dev/null 2>&1 || true
exit $status
