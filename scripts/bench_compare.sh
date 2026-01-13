#!/bin/bash
# Run benchmarks and compare against baseline

set -e

BASELINE_NAME=${1:-master}

echo "📊 Running benchmarks against baseline: $BASELINE_NAME"
echo ""

# Check if baseline exists
if [ ! -d "target/criterion" ]; then
    echo "⚠️  No previous benchmark data found."
    echo "   Run 'make bench-baseline' first to create a baseline."
    echo ""
    exit 1
fi

# Run all benchmarks and compare
echo "Running benchmarks (this may take a few minutes)..."
echo ""

cargo bench --no-fail-fast -- --baseline "$BASELINE_NAME"

echo ""
echo "✅ Benchmark complete!"
echo ""
echo "📈 View detailed report:"
echo "   open target/criterion/report/index.html"
echo ""
echo "📊 Summary of changes:"
echo ""

# Try to extract summary from criterion output
if [ -f "target/criterion/report/index.html" ]; then
    echo "   Check the HTML report for detailed comparisons"
else
    echo "   Run benchmarks again if report wasn't generated"
fi

echo ""
echo "💡 Tips:"
echo "   - Look for 'change:' lines showing % improvement/regression"
echo "   - Green = improvement, Red = regression"
echo "   - Focus on p95 latencies (more important than mean for Lambda)"
echo ""
echo "🔄 To update baseline after changes:"
echo "   make bench-baseline"
