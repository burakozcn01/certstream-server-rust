#!/bin/bash

LOGFILE="/tmp/certstream_stats_$(date +%Y%m%d_%H%M%S).csv"
DURATION_HOURS=${1:-3}
INTERVAL_SECS=${2:-30}

TOTAL_SECS=$((DURATION_HOURS * 3600))
ITERATIONS=$((TOTAL_SECS / INTERVAL_SECS))

echo "timestamp,cpu_percent,mem_usage_mb,mem_percent,net_rx_mb,net_tx_mb,messages_sent,ct_logs" > "$LOGFILE"

echo "Monitoring certstream-test for $DURATION_HOURS hours"
echo "Interval: ${INTERVAL_SECS}s"
echo "Log file: $LOGFILE"
echo ""

for i in $(seq 1 $ITERATIONS); do
    TIMESTAMP=$(date "+%Y-%m-%d %H:%M:%S")

    STATS=$(docker stats certstream-test --no-stream --format "{{.CPUPerc}},{{.MemUsage}},{{.MemPerc}},{{.NetIO}}" 2>/dev/null)

    if [ -z "$STATS" ]; then
        echo "[$TIMESTAMP] Container not running"
        sleep $INTERVAL_SECS
        continue
    fi

    CPU=$(echo "$STATS" | cut -d',' -f1 | tr -d '%')
    MEM_RAW=$(echo "$STATS" | cut -d',' -f2)
    MEM_MB=$(echo "$MEM_RAW" | cut -d'/' -f1 | tr -d 'MiB ' | tr -d 'GiB')
    MEM_PCT=$(echo "$STATS" | cut -d',' -f3 | tr -d '%')
    NET_RAW=$(echo "$STATS" | cut -d',' -f4)
    NET_RX=$(echo "$NET_RAW" | cut -d'/' -f1 | tr -d ' ')
    NET_TX=$(echo "$NET_RAW" | cut -d'/' -f2 | tr -d ' ')

    METRICS=$(curl -s http://localhost:8080/metrics 2>/dev/null)
    MSGS=$(echo "$METRICS" | grep "^certstream_messages_sent" | awk '{print $2}')
    LOGS=$(echo "$METRICS" | grep "^certstream_ct_logs_count" | awk '{print $2}')

    [ -z "$MSGS" ] && MSGS="0"
    [ -z "$LOGS" ] && LOGS="0"

    echo "$TIMESTAMP,$CPU,$MEM_MB,$MEM_PCT,$NET_RX,$NET_TX,$MSGS,$LOGS" >> "$LOGFILE"

    ELAPSED=$((i * INTERVAL_SECS / 60))
    REMAINING=$(((ITERATIONS - i) * INTERVAL_SECS / 60))
    echo "[$TIMESTAMP] CPU: ${CPU}% | MEM: ${MEM_MB}MB | Messages: $MSGS | Elapsed: ${ELAPSED}m | Remaining: ${REMAINING}m"

    sleep $INTERVAL_SECS
done

echo ""
echo "Monitoring complete!"
echo "Log file: $LOGFILE"
echo ""
echo "=== Summary ==="
echo "Total records: $(wc -l < "$LOGFILE")"
echo ""
echo "CPU Stats:"
tail -n +2 "$LOGFILE" | cut -d',' -f2 | sort -n | awk '{a[NR]=$1} END {print "  Min: " a[1] "% | Max: " a[NR] "% | Avg: " (a[int(NR/2)]) "%"}'
echo ""
echo "Memory Stats:"
tail -n +2 "$LOGFILE" | cut -d',' -f3 | sort -n | awk '{a[NR]=$1} END {print "  Min: " a[1] "MB | Max: " a[NR] "MB | Avg: " (a[int(NR/2)]) "MB"}'
