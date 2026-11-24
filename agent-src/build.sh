#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
AGENT_SRC="$ROOT_DIR/agent-src"
BUILD_DIR="$AGENT_SRC/build"
CLASSES_DIR="$BUILD_DIR/classes"
TEST_CLASSES_DIR="$BUILD_DIR/test-classes"
OUT_JAR="$ROOT_DIR/agent/openidea-spring-agent.jar"

rm -rf "$BUILD_DIR"
mkdir -p "$CLASSES_DIR" "$TEST_CLASSES_DIR" "$ROOT_DIR/agent"

RELEASE_FLAG="--release 8"
if ! javac --release 8 -version >/dev/null 2>&1; then
  # Verify whether JDK supports --release 8 cleanly by probing a trivial compile
  :
fi

# Compile agent classes (JDK 8 bytecode compatibility when supported, fallback to 11 if JDK removed 8)
JAVA_FILES=$(find "$AGENT_SRC/src" -name "*.java" | sort)
if javac --release 8 -Xlint:-options -encoding UTF-8 -d "$CLASSES_DIR" $JAVA_FILES 2>/dev/null; then
  RELEASE_FLAG="--release 8 -Xlint:-options"
else
  echo "javac --release 8 unavailable, falling back to --release 11"
  RELEASE_FLAG="--release 11"
  javac --release 11 -encoding UTF-8 -d "$CLASSES_DIR" $JAVA_FILES
fi

cat > "$BUILD_DIR/MANIFEST.MF" <<'EOF'
Manifest-Version: 1.0
Premain-Class: io.openidea.spring.agent.SpringProbeAgent
Can-Redefine-Classes: false
Can-Retransform-Classes: false
Implementation-Title: OpenIDEA Spring Runtime Probe Agent
Implementation-Version: 0.1.0
EOF

jar cfm "$OUT_JAR" "$BUILD_DIR/MANIFEST.MF" -C "$CLASSES_DIR" .

# Compile and run self-tests
TEST_FILES=$(find "$AGENT_SRC/test" -name "*.java" | sort)
javac $RELEASE_FLAG -encoding UTF-8 -cp "$CLASSES_DIR" -d "$TEST_CLASSES_DIR" $TEST_FILES
java -cp "$CLASSES_DIR:$TEST_CLASSES_DIR" io.openidea.spring.agent.AgentSelfTest

echo "Built $OUT_JAR ($RELEASE_FLAG)"
