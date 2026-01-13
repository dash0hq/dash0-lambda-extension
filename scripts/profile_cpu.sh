#!/bin/bash
# Generate flamegraph for critical benchmark

set -e

echo "🔥 Generating CPU flamegraph..."
echo ""

# Check if flamegraph is installed
if ! command -v flamegraph &> /dev/null; then
    echo "⚠️  flamegraph not found. Install with:"
    echo "    cargo install flamegraph"
    echo ""
    exit 1
fi

# Create profiling output directory
mkdir -p target/profiling

# Select benchmark to profile (default: store_cloning)
BENCH="${1:-store_cloning}"

echo "Profiling benchmark: $BENCH"
echo "This may take a few minutes..."
echo ""

# Profile the benchmark
cargo flamegraph --bench "$BENCH" -- --bench \
    --profile-time 10 \
    -o target/profiling/flamegraph-${BENCH}.svg

echo ""
echo "✅ Flamegraph saved to target/profiling/flamegraph-${BENCH}.svg"
echo "   Open in browser to view:"
echo "   open target/profiling/flamegraph-${BENCH}.svg"
echo ""
echo "💡 To profile a different benchmark:"
echo "   ./scripts/profile_cpu.sh store_mutex"
echo "   ./scripts/profile_cpu.sh http_client"
echo "   ./scripts/profile_cpu.sh protobuf_ops"
