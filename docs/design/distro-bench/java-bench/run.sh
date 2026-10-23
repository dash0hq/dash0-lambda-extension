#!/usr/bin/env bash
# Usage: run.sh <path to sig-701-java-masking worktree>. Copies the bench in, runs it, removes it again.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
DISTRO=$1/opt/java/opentelemetry-java-distro
FIX=$HERE/../../../../benches/fixtures/masking-payloads.json
OUT=$(mktemp -d)
python3 "$HERE/fixtures-to-properties.py" "$FIX" "$OUT/fixtures.properties"
DST=$DISTRO/custom/src/test/java/io/dash0/javaagent/masking/DistroBench.java
cp "$HERE/DistroBench.java" "$DST"
trap 'rm -f "$DST" "$DISTRO"/custom/build/classes/java/test/io/dash0/javaagent/masking/DistroBench*.class' EXIT
(cd "$DISTRO" && CP_OUT=$OUT/cp.txt ./gradlew -q -I "$HERE/cp.gradle" :custom:printBenchCp)
CP=$(cat "$OUT/cp.txt")
for m in defaults custom; do java -cp "$CP" io.dash0.javaagent.masking.DistroBench "$OUT/fixtures.properties" correctness $m; done > "$HERE/correctness.txt"
for m in defaults custom; do java -Xms512m -cp "$CP" io.dash0.javaagent.masking.DistroBench "$OUT/fixtures.properties" warm $m; done | tee "$HERE/warm.txt"
for m in defaults custom; do for s in small_json json_4k; do for i in 1 2 3 4 5; do
  java -cp "$CP" io.dash0.javaagent.masking.DistroBench "$OUT/fixtures.properties" cold $m $s
done; done; done | tee "$HERE/cold.txt"
