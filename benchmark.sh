#!/bin/bash
# jude vs hey — comprehensive benchmark
set -e

JUDE="$(dirname "$0")/target/release/jude"
HEY="/home/abraham/.local/bin/hey"
SERVER="$(dirname "$0")/bench-server/target/release/bench-server"
PORT=9977

# Payload sizes to test
PAYLOADS=(0 1024 10240 102400)
PAYLOAD_NAMES=("0B" "1KB" "10KB" "100KB")

# Concurrency levels
CONCURRENCIES=(1 10 50 100 200 500 1000 2000 5000 10000)

# Requests per test (scale with concurrency)
get_n() {
    local c=$1
    if [ $c -le 100 ]; then echo 10000
    elif [ $c -le 1000 ]; then echo 20000
    elif [ $c -le 5000 ]; then echo 30000
    else echo 50000
    fi
}

echo "================================================================"
echo "  jude vs hey — Comprehensive Benchmark"
echo "  $(date)"
echo "  Machine: $(nproc) cores, $(free -h | awk '/Mem:/{print $2}') RAM"
echo "  FD limit: $(ulimit -n)"
echo "================================================================"
echo ""

for pi in "${!PAYLOADS[@]}"; do
    payload=${PAYLOADS[$pi]}
    pname=${PAYLOAD_NAMES[$pi]}

    echo "================================================================"
    echo "  Payload: $pname ($payload bytes)"
    echo "================================================================"

    # Start server
    $SERVER $PORT $payload &
    SERVER_PID=$!
    sleep 0.5

    URL="http://localhost:$PORT/"

    printf "%-8s | %-12s %-12s | %-10s %-10s | %-10s %-10s | %-10s %-10s | %-8s %-8s\n" \
        "conc" "hey_rps" "jude_rps" "hey_p50" "jude_p50" "hey_p99" "jude_p99" "hey_mem" "jude_mem" "hey_err" "jude_err"
    echo "---------|------------------------------|----------------------|----------------------|----------------------|-----------------"

    for c in "${CONCURRENCIES[@]}"; do
        n=$(get_n $c)

        # Ensure n >= c
        if [ $n -lt $c ]; then n=$c; fi

        # Run hey
        hey_out=$($HEY -n $n -c $c $URL 2>/dev/null)
        hey_rps=$(echo "$hey_out" | grep "Requests/sec" | awk '{print $2}')
        hey_p50=$(echo "$hey_out" | grep "50% in" | awk '{print $3}')
        hey_p99=$(echo "$hey_out" | grep "99% in" | awk '{print $3}')
        hey_err=$(echo "$hey_out" | grep -c "Error\|error" || true)

        hey_mem=$(/usr/bin/time -v $HEY -n $n -c $c $URL 2>&1 | grep "Maximum resident" | awk '{print $NF}')

        # Run jude
        jude_out=$($JUDE -n $n -c $c $URL 2>/dev/null)
        jude_rps=$(echo "$jude_out" | grep "Requests/sec" | awk '{print $2}')
        jude_p50=$(echo "$jude_out" | grep "50% in" | awk '{print $3}')
        jude_p99=$(echo "$jude_out" | grep "99% in" | awk '{print $3}')
        jude_err_line=$(echo "$jude_out" | grep "Errors:" | awk '{print $2}')
        jude_err=${jude_err_line:-0}

        jude_mem=$(/usr/bin/time -v $JUDE -n $n -c $c $URL 2>&1 | grep "Maximum resident" | awk '{print $NF}')

        printf "%-8s | %-12s %-12s | %-10s %-10s | %-10s %-10s | %-10s %-10s | %-8s %-8s\n" \
            "c=$c" "$hey_rps" "$jude_rps" "${hey_p50}s" "${jude_p50}s" "${hey_p99}s" "${jude_p99}s" "${hey_mem}KB" "${jude_mem}KB" "$hey_err" "$jude_err"
    done

    echo ""

    # Stop server
    kill $SERVER_PID 2>/dev/null
    wait $SERVER_PID 2>/dev/null
    sleep 0.3
done

echo "================================================================"
echo "  Binary size comparison"
echo "================================================================"
echo "  hey:  $(ls -lh $HEY | awk '{print $5}')"
echo "  jude: $(ls -lh $JUDE | awk '{print $5}')"
echo ""
echo "Done."
