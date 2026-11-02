#!/bin/bash
# Dry-run tests for the root helper. No root needed, nothing is changed on the host.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
helper=$here/../helper/interweave-helper
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

mkdir "$tmp/bin"
cat >"$tmp/bin/nmcli" <<'NM'
#!/bin/bash
case $5 in eth0) echo 192.168.1.1 ;; wlan0) echo 10.0.0.1 ;; esac
NM
chmod +x "$tmp/bin/nmcli"
export PATH=$tmp/bin:$PATH DRY_RUN=1 INTERWEAVE_CONF=$tmp/links.conf
fails=0

expect_ok() {
    local name=$1 input=$2 want=$3 got
    got=$(printf '%s\n' "$input" | "$helper" save 2>&1 | grep -v ' del \|policy=0')
    if [[ $got != "$want" ]]; then
        echo "FAIL $name"; diff <(echo "$want") <(echo "$got"); fails=$((fails + 1))
    else
        echo "ok   $name"
    fi
}

expect_reject() {
    local name=$1 input=$2
    rm -f "$INTERWEAVE_CONF"
    if printf '%s\n' "$input" | "$helper" save >/dev/null 2>&1 || [[ -e $INTERWEAVE_CONF ]]; then
        echo "FAIL $name (accepted)"; fails=$((fails + 1))
    else
        echo "ok   $name"
    fi
}

expect_ok "two links, caps on one" "eth0 2 0 0
wlan0 1 50 10" "tc qdisc replace dev wlan0 root cake bandwidth 10mbit
ip link add ifbiw1 type ifb
ip link set ifbiw1 up
tc qdisc replace dev wlan0 handle ffff: ingress
tc filter replace dev wlan0 parent ffff: protocol all matchall action mirred egress redirect dev ifbiw1
tc qdisc replace dev ifbiw1 root cake bandwidth 50mbit ingress
sysctl -qw net.ipv4.fib_multipath_hash_policy=1
ip route replace default metric 10 nexthop via 192.168.1.1 dev eth0 weight 2 nexthop via 10.0.0.1 dev wlan0 weight 1
ip -6 route replace unreachable default metric 10"

expect_ok "offline link is skipped, no multipath with one link" "eth0 1 0 0
usb0 1 0 0" ""

expect_ok "empty config turns everything off" "" ""

expect_ok "comments and blank lines ignored" "# hi

eth0 1 0 5" "tc qdisc replace dev eth0 root cake bandwidth 5mbit"

expect_reject "shell injection in device" 'eth0;rm${IFS}-rf 1 0 0'
expect_reject "device name too long" "abcdefghijklmnop 1 0 0"
expect_reject "weight zero" "eth0 0 0 0"
expect_reject "weight too big" "eth0 257 0 0"
expect_reject "missing field" "eth0 1 0"
expect_reject "extra field" "eth0 1 0 0 0"
expect_reject "negative cap" "eth0 1 -5 0"

mkdir "$tmp/rules"
me=$(id -nu)
if PKEXEC_UID=$(id -u) INTERWEAVE_RULES_DIR=$tmp/rules "$helper" grant \
    && grep -q "subject.user == \"$me\"" "$tmp/rules/50-interweave-$me.rules"; then
    echo "ok   grant writes a rule for the caller"
else
    echo "FAIL grant writes a rule for the caller"; fails=$((fails + 1))
fi
if "$helper" grant 2>/dev/null; then
    echo "FAIL grant without pkexec (accepted)"; fails=$((fails + 1))
else
    echo "ok   grant without pkexec"
fi

((fails == 0)) && echo "all passed" || { echo "$fails failed"; exit 1; }
