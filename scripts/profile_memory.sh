#!/bin/bash
# Run DHAT memory profiler on benchmarks

set -e

echo "📊 Running memory profiler..."
echo ""

# Check if valgrind is available (optional, for more detailed analysis)
if command -v valgrind &> /dev/null; then
    echo "✓ Valgrind found - will use for detailed memory analysis"
    USE_VALGRIND=true
else
    echo "⚠️  Valgrind not found - using DHAT only"
    echo "   Install valgrind for more detailed analysis:"
    echo "   brew install valgrind  # macOS"
    echo "   apt-get install valgrind  # Linux"
    USE_VALGRIND=false
fi

echo ""

# Create profiling output directory
mkdir -p target/profiling

# Select benchmark to profile (default: store_cloning)
BENCH="${1:-store_cloning}"

echo "Profiling memory for benchmark: $BENCH"
echo ""

# Note: DHAT integration requires code changes to enable the allocator
# For now, we'll use valgrind massif if available

if [ "$USE_VALGRIND" = true ]; then
    echo "Running Valgrind Massif..."

    # Build benchmark in release mode
    cargo build --release --bench "$BENCH"

    # Run with massif
    valgrind --tool=massif \
        --massif-out-file=target/profiling/massif-${BENCH}.out \
        --time-unit=B \
        ./target/release/deps/"$BENCH"-* --bench || true

    echo ""
    echo "✅ Massif profile saved to target/profiling/massif-${BENCH}.out"
    echo "   View with:"
    echo "   ms_print target/profiling/massif-${BENCH}.out"
else
    echo "📝 To enable DHAT memory profiling:"
    echo "1. Add to benchmark file:"
    echo "   use dhat::{Dhat, DhatAlloc};"
    echo "   #[global_allocator]"
    echo "   static ALLOCATOR: DhatAlloc = DhatAlloc;"
    echo ""
    echo "2. Run benchmark normally:"
    echo "   cargo bench --bench $BENCH"
    echo ""
    echo "3. View DHAT output in target/dhat/"
fi

echo ""
echo "💡 For allocation tracking, consider also using:"
echo "   cargo instruments -t Allocations --bench $BENCH  # macOS only"
