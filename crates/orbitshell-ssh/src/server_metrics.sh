export LC_ALL=C
export PATH=/usr/bin:/bin:/usr/sbin:/sbin

[ "$(uname -s)" = Linux ] || exit 1
printf 'orbitshell-metrics-v1\n'
awk '$1 == "cpu" && NF >= 5 { print; exit }' /proc/stat 2>/dev/null
awk '/^Mem(Total|Available):/ { print $1, $2 }' /proc/meminfo 2>/dev/null
df -Pk / 2>/dev/null | awk 'NR == 2 && NF >= 6 { print "disk", $(NF-4), $(NF-3) }'

if [ "$1" = inventory ]; then
    model=$(awk -F: '/^(model name|Hardware|uarch)[[:space:]]*:/ { sub(/^[[:space:]]+/, "", $2); print $2; exit }' /proc/cpuinfo 2>/dev/null)
    printf 'model %s\n' "${model:-$(uname -m)}"
    printf 'threads %s\n' "$(getconf _NPROCESSORS_ONLN 2>/dev/null)"

    if [ -r /sys/bus/pci/devices ]; then
        count=0
        for device in /sys/bus/pci/devices/*; do
            [ -r "$device/class" ] || continue
            read -r class < "$device/class" || continue
            case "$class" in 0x03*) ;; *) continue ;; esac
            count=$((count + 1))
            [ "$count" -le 16 ] || continue

            slot=${device##*/}
            description=$(lspci -D -nn -s "$slot" 2>/dev/null)
            if [ -n "$description" ]; then
                description=${description#* }
                description=${description#*: }
            elif [ -r "$device/vendor" ] && [ -r "$device/device" ]; then
                read -r vendor < "$device/vendor"
                read -r identifier < "$device/device"
                description="PCI $vendor:$identifier"
            fi
            printf 'gpu %s\n' "${description:-PCI $slot}"
        done
        printf 'gpu-count %s\n' "$count"
    fi
fi
exit 0
