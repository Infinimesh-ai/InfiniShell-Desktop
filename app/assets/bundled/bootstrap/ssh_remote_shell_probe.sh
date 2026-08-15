function warp_remote_shell_from_probe_output() {
    local probe_output="$1"
    local marker="__WARP_REMOTE_SHELL__"
    local line
    local remote_shell=""

    while IFS= read -r line; do
        case "$line" in
            "$marker"*) remote_shell="${line#"$marker"}" ;;
        esac
    done <<< "$probe_output"

    printf '%s' "${remote_shell%$'\r'}"
}

function warp_remote_shell_supports_bootstrap() {
    case "$1" in
        bash | */bash | zsh | */zsh) return 0 ;;
        *) return 1 ;;
    esac
}
