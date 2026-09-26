#!/usr/bin/env bash
set -euo pipefail

# Run on a disposable Ubuntu 24.04 HWE (Linux >=6.12) VM with bubblewrap,
# e2fsprogs, quota, and python3 installed. Build the standalone lib tests first:
# cargo test -p flow-like-standalone --lib --no-run
# sudo scripts/test-standalone-isolation-linux.sh --disposable-linux-host \
#   target/debug/deps/flow_like_standalone-<test executable hash>
# The harness creates a private loop filesystem and cgroup, then removes them.
# It never changes existing mounts, quota assignments, or controller settings.

if [[ $# != 2 || $1 != --disposable-linux-host || $(uname -s) != Linux || $EUID != 0 ]]; then
    echo 'Usage (root, disposable Linux VM): test-standalone-isolation-linux.sh --disposable-linux-host <standalone-lib-test-executable>' >&2
    exit 2
fi
test_binary=$(realpath -- "$2")
[[ -f $test_binary && -x $test_binary ]] || exit 2
for tool in mkfs.ext4 mount umount mountpoint setquota chattr python3 timeout; do
    command -v "$tool" >/dev/null
done
[[ -x /usr/bin/bwrap ]] || { echo 'Install bubblewrap first.' >&2; exit 2; }
for controller in cpu memory pids; do
    [[ " $(cat /sys/fs/cgroup/cgroup.subtree_control) " == *" $controller "* ]] || {
        echo "The VM must already delegate the $controller controller at /sys/fs/cgroup." >&2
        exit 2
    }
done

fixture_root=$(mktemp -d /var/tmp/flow-like-isolation.XXXXXXXX)
cgroup_root="/sys/fs/cgroup/flow-like-isolation-test-$$"
cleanup() {
    local status=$?
    trap - EXIT
    if [[ -d $cgroup_root ]]; then
        printf '1' > "$cgroup_root/cgroup.kill" || true
        for _ in {1..20}; do
            for group in "$cgroup_root"/p*; do
                [[ -d $group ]] && rmdir -- "$group" 2>/dev/null || true
            done
            rmdir -- "$cgroup_root" 2>/dev/null && break
            sleep 0.1
        done
    fi
    if mountpoint -q "$fixture_root/mount"; then
        umount -- "$fixture_root/mount" || {
            echo "Cleanup could not unmount $fixture_root/mount; fixture retained." >&2
            exit 1
        }
    fi
    rm -rf -- "$fixture_root"
    exit "$status"
}
trap cleanup EXIT
umask 077
truncate -s 256M "$fixture_root/quota.ext4"
mkfs.ext4 -q -F -O project,quota -E quotatype=prjquota "$fixture_root/quota.ext4"
mkdir "$fixture_root/mount"
mount -o loop,prjquota "$fixture_root/quota.ext4" "$fixture_root/mount"
data="$fixture_root/mount/state/placement-data/acceptance/current/store"
mkdir -p "$data"
chattr -p 314159 +P "$data"
setquota -P 314159 16384 16384 4096 4096 "$fixture_root/mount"

mkdir "$cgroup_root"
printf '400000 100000' > "$cgroup_root/cpu.max"
printf '1073741824' > "$cgroup_root/memory.max"
printf '256' > "$cgroup_root/pids.max"
printf '+cpu +memory +pids' > "$cgroup_root/cgroup.subtree_control"
export FLOW_LIKE_DEVICE_CGROUP_ROOT="$cgroup_root"
export FLOW_LIKE_ISOLATION_TEST_ROOT="$fixture_root/mount"
timeout --kill-after=5s 150s "$test_binary" \
    --exact isolation::linux::tests::linux_boundary_acceptance --ignored --nocapture
