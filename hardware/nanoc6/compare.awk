NR == FNR {
    if ($1 == "RENVO_HW" && $2 ~ /^case_[0-9][0-9][0-9][0-9]$/) {
        actual[$2] = tolower($3)
    }
    next
}

FNR == 1 {
    next
}

$1 in actual {
    expected = tolower($5)
    sub(/^0x/, "", expected)
    ++tested
    if (actual[$1] != expected) {
        ++failed
        printf "%s expected=%s hardware=%s\n", $1, expected, actual[$1]
    }
}

END {
    printf "NanoC6 hardware: %d/%d cases matched\n", tested - failed, tested
    if (tested != 40 || failed != 0) {
        exit 1
    }
}
