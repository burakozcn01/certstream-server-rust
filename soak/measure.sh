#!/usr/bin/env bash
# Samples /metrics over a run and reports how current and how complete the
# stream was, alongside CPU and RSS.
#
#   bash soak/measure.sh [duration_seconds] [sample_interval_seconds]
#
# Env:
#   BASE  metrics endpoint base URL (default http://localhost:8080)
#   PID   process to sample CPU/RSS from (default: discovered by name)
#   OUT   directory for the raw samples (default: a mktemp dir)
#
# Throughput and resource use answer "what does it cost". They do not answer
# "how current and how complete is my view of the CT ecosystem", which is the
# question a consumer actually has. These are reported together, from one run.

set -u
BASE=${BASE:-http://localhost:8080}
DURATION=${1:-10800}
INTERVAL=${2:-30}
OUT=${OUT:-$(mktemp -d)}
PID=${PID:-$(pgrep -f 'certstream-server-rust' | head -1)}

SAMPLES="$OUT/samples.tsv"
mkdir -p "$OUT"

# value of a bare (unlabelled) metric
m() { awk -v k="$1" '$1==k {print $2; exit}' "$2"; }
# a metric family that may or may not carry labels, summed either way
sum_family() { awk -v k="$1" '$1==k {s+=$2; next} index($0, k"{")==1 {s+=$NF} END {print s+0}' "$2"; }
# sum / max / count over a labelled metric family
agg() { awk -v k="$1" -v op="$2" '
    index($0, k"{")==1 {
        v=$NF+0; n++; s+=v; if (n==1 || v>mx) mx=v;
    }
    END {
        if (op=="sum") print s+0;
        else if (op=="max") print mx+0;
        else if (op=="count") print n+0;
    }' "$3"; }

# Median of a labelled family, and how many members exceed a threshold.
#
# The median rather than the mean, and a count rather than the max, because a
# log that has stopped issuing skews both: its newest entry really is months
# old, which says nothing about how far behind this server is. The median says
# how fresh a typical log's view is; the count says how many are actually
# stale.
quantile() { awk -v k="$1" -v op="$2" -v t="$3" '
    index($0, k"{")==1 { v[n++] = $NF+0; if ($NF+0 > t) over++ }
    END {
        if (op=="over") { print over+0; exit }
        if (n == 0) { print 0; exit }
        # Insertion sort: portable across awk implementations (asort is a gawk
        # extension), and n here is the number of CT logs.
        for (i = 1; i < n; i++) {
            key = v[i]
            for (j = i - 1; j >= 0 && v[j] > key; j--) v[j+1] = v[j]
            v[j+1] = key
        }
        print (n % 2) ? v[int(n/2)] : (v[n/2 - 1] + v[n/2]) / 2
    }' "$4"; }

if [[ -z ${PID} ]]; then
    echo "no certstream-server-rust process found; set PID=" >&2
    exit 1
fi

echo "measuring pid $PID against $BASE for ${DURATION}s every ${INTERVAL}s"
echo "raw samples: $SAMPLES"

printf 'ts\tcpu_pct\trss_kb\tmsgs_sent\tbytes_sent\tlag_entries_max\tlogs_behind_5k\tdelay_p50\tdelay_over_60s\tdelay_max\tws_lagged\tsse_lagged\tws_cut\tsse_cut\tdupes\n' > "$SAMPLES"

START=$(date +%s)
END=$((START + DURATION))
SNAP="$OUT/metrics.txt"

while [[ $(date +%s) -lt $END ]]; do
    if ! curl -sf -m 10 "$BASE/metrics" -o "$SNAP"; then
        echo "$(date -u +%H:%M:%S) metrics scrape failed" >&2
        sleep "$INTERVAL"
        continue
    fi

    read -r CPU RSS <<<"$(ps -o %cpu=,rss= -p "$PID" 2>/dev/null || echo '0 0')"

    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$(date +%s)" "${CPU:-0}" "${RSS:-0}" \
        "$(sum_family certstream_messages_sent "$SNAP")" \
        "$(agg certstream_bytes_sent_total sum "$SNAP")" \
        "$(agg certstream_ct_log_lag_entries max "$SNAP")" \
        "$(awk 'index($0,"certstream_ct_log_lag_entries{")==1 && $NF+0>5000 {n++} END{print n+0}' "$SNAP")" \
        "$(quantile certstream_ct_log_ingest_delay_seconds p50 0 "$SNAP")" \
        "$(quantile certstream_ct_log_ingest_delay_seconds over 60 "$SNAP")" \
        "$(agg certstream_ct_log_ingest_delay_seconds max "$SNAP")" \
        "$(m certstream_ws_messages_lagged "$SNAP")" \
        "$(m certstream_sse_messages_lagged "$SNAP")" \
        "$(m certstream_ws_disconnect_lag "$SNAP")" \
        "$(m certstream_sse_disconnect_lag "$SNAP")" \
        "$(m certstream_duplicates_filtered "$SNAP")" \
        >> "$SAMPLES"

    sleep "$INTERVAL"
done

echo
echo "================ run summary ================"
awk -F'\t' '
NR==1 { next }
{
    n++
    cpu+=$2; if ($2+0>cpu_max) cpu_max=$2+0
    rss+=$3; if ($3+0>rss_max) rss_max=$3+0
    if (n==1) { t0=$1; msg0=$4; by0=$5; wl0=$11; sl0=$12; wc0=$13; sc0=$14; d0=$15 }
    t1=$1; msg1=$4; by1=$5; wl1=$11; sl1=$12; wc1=$13; sc1=$14; d1=$15
    lag_max=($6+0>lag_max)?$6+0:lag_max
    behind+=$7; if ($7+0>behind_max) behind_max=$7+0
    p50+=$8; if ($8+0>p50_max) p50_max=$8+0
    stale+=$9; if ($9+0>stale_max) stale_max=$9+0
}
END {
    if (n<2) { print "not enough samples"; exit 1 }
    dt = t1-t0
    printf "window                       %d samples over %.1f min\n", n, dt/60
    printf "\n-- cost --\n"
    printf "cpu                          %.1f%% mean, %.1f%% peak\n", cpu/n, cpu_max
    printf "rss                          %.1f MB mean, %.1f MB peak\n", (rss/n)/1024, rss_max/1024
    printf "delivered throughput         %.1f msg/s\n", (msg1-msg0)/dt
    printf "outbound                     %.2f MB total, %.1f KB/s\n", (by1-by0)/1048576, ((by1-by0)/dt)/1024
    printf "duplicates filtered          %d\n", d1-d0
    printf "\n-- how current --\n"
    printf "ingest delay, median log     %.1f s mean, %.1f s worst sample\n", p50/n, p50_max
    printf "logs >60 s stale             %.1f mean, %d peak\n", stale/n, stale_max
    printf "entries behind head          %.0f worst log\n", lag_max
    printf "logs >5k entries behind      %.1f mean, %d peak\n", behind/n, behind_max
    printf "\n-- how complete --\n"
    printf "ws messages missed           %d\n", wl1-wl0
    printf "sse messages missed          %d\n", sl1-sl0
    printf "subscribers cut for lag      %d ws, %d sse\n", wc1-wc0, sc1-sc0
}' "$SAMPLES"
echo "============================================="
echo "raw: $SAMPLES"
