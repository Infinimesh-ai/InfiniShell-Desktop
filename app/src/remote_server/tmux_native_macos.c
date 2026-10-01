#include <errno.h>
#include <libproc.h>
#include <stdint.h>
#include <string.h>
#include <sys/proc_info.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

/* Darwin 结构始终由系统 SDK 定义；仅这两个扁平结果与 Rust 共享。 */
struct isp_tmux_process {
    int32_t pid;
    int32_t parent;
    uint32_t uid;
    int32_t group;
    int32_t foreground_group;
    int32_t session;
    uint64_t tty;
    uint32_t image_length;
    uint32_t cwd_length;
    uint8_t image[4096];
    uint8_t cwd[4096];
};

struct isp_tmux_socket {
    int32_t descriptor;
    uint32_t listening;
    uint64_t socket;
    uint64_t protocol;
    uint64_t peer_socket;
    uint64_t peer_protocol;
    uint32_t local_length;
    uint32_t peer_length;
    uint8_t local_path[256];
    uint8_t peer_path[256];
};

int isp_tmux_process_snapshot(int32_t pid, struct isp_tmux_process *out, uint32_t size) {
    if (pid <= 0 || out == NULL || size != sizeof(*out)) return EINVAL;
    memset(out, 0, sizeof(*out));
    struct proc_bsdinfo info;
    struct proc_vnodepathinfo paths;
    memset(&info, 0, sizeof(info));
    memset(&paths, 0, sizeof(paths));
    if (proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, sizeof(info)) != (int)sizeof(info)) return errno ? errno : EIO;
    if (info.pbi_pid != (uint32_t)pid || info.pbi_uid != geteuid()) return EPERM;
    if (proc_pidpath(pid, out->image, sizeof(out->image)) <= 0) return errno ? errno : EIO;
    if (proc_pidinfo(pid, PROC_PIDVNODEPATHINFO, 0, &paths, sizeof(paths)) != (int)sizeof(paths)) return errno ? errno : EIO;
    size_t image_length = strnlen((const char *)out->image, sizeof(out->image));
    size_t cwd_length = strnlen(paths.pvi_cdir.vip_path, sizeof(paths.pvi_cdir.vip_path));
    if (!image_length || image_length >= sizeof(out->image) || !cwd_length
        || cwd_length >= sizeof(paths.pvi_cdir.vip_path) || cwd_length >= sizeof(out->cwd)) return EOVERFLOW;
    out->pid = pid;
    out->parent = (int32_t)info.pbi_ppid;
    out->uid = info.pbi_uid;
    out->group = (int32_t)info.pbi_pgid;
    out->foreground_group = (int32_t)info.e_tpgid;
    out->session = getsid(pid);
    if (out->session <= 0) return errno ? errno : ESRCH;
    out->tty = info.e_tdev;
    out->image_length = (uint32_t)image_length;
    out->cwd_length = (uint32_t)cwd_length;
    memcpy(out->cwd, paths.pvi_cdir.vip_path, cwd_length);
    return 0;
}

int isp_tmux_list_pids(int32_t group, int32_t *out, uint32_t capacity, uint32_t *count) {
    if (out == NULL || count == NULL || capacity == 0 || capacity > 2048 || group < 0) return EINVAL;
    *count = 0;
    memset(out, 0, capacity * sizeof(*out));
    int bytes = proc_listpids(group ? PROC_PGRP_ONLY : PROC_UID_ONLY,
                             group ? (uint32_t)group : geteuid(), out, (int)(capacity * sizeof(*out)));
    if (bytes <= 0 || (size_t)bytes >= capacity * sizeof(*out) || (size_t)bytes % sizeof(*out)) return errno ? errno : EOVERFLOW;
    *count = (uint32_t)(bytes / sizeof(*out));
    return 0;
}

static int copy_path(const struct sockaddr_un *address, uint8_t *out, uint32_t *length) {
    size_t prefix = __builtin_offsetof(struct sockaddr_un, sun_path);
    *length = 0;
    if (address->sun_len == 0) return 0;
    if (address->sun_family != AF_UNIX || address->sun_len < prefix
        || address->sun_len > sizeof(*address)) return EINVAL;
    size_t size = strnlen(address->sun_path, address->sun_len - prefix);
    if (size > sizeof(address->sun_path)) return EOVERFLOW;
    memcpy(out, address->sun_path, size);
    *length = (uint32_t)size;
    return 0;
}

int isp_tmux_socket_inventory(int32_t pid, struct isp_tmux_socket *out, uint32_t size,
                             uint32_t capacity, uint32_t *count) {
    if (pid <= 0 || out == NULL || count == NULL || size != sizeof(*out)
        || capacity == 0 || capacity > 1024) return EINVAL;
    *count = 0;
    memset(out, 0, capacity * sizeof(*out));
    struct proc_fdinfo descriptors[1024];
    memset(descriptors, 0, sizeof(descriptors));
    int bytes = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, descriptors, sizeof(descriptors));
    if (bytes <= 0 || (size_t)bytes >= sizeof(descriptors) || (size_t)bytes % sizeof(descriptors[0])) return errno ? errno : EOVERFLOW;
    for (int index = 0; index < bytes / (int)sizeof(descriptors[0]); index++) {
        if (descriptors[index].proc_fdtype != PROX_FDTYPE_SOCKET) continue;
        struct socket_fdinfo socket;
        memset(&socket, 0, sizeof(socket));
        if (proc_pidfdinfo(pid, descriptors[index].proc_fd, PROC_PIDFDSOCKETINFO,
                           &socket, sizeof(socket)) != (int)sizeof(socket)) return errno ? errno : EIO;
        if (socket.psi.soi_family != AF_UNIX || socket.psi.soi_type != SOCK_STREAM
            || socket.psi.soi_kind != SOCKINFO_UN) continue;
        if (*count >= capacity) return EOVERFLOW;
        struct isp_tmux_socket *entry = &out[(*count)++];
        const struct un_sockinfo *local = &socket.psi.soi_proto.pri_un;
        entry->descriptor = descriptors[index].proc_fd;
        entry->listening = (socket.psi.soi_options & SO_ACCEPTCONN) != 0;
        entry->socket = socket.psi.soi_so;
        entry->protocol = socket.psi.soi_pcb;
        entry->peer_socket = local->unsi_conn_so;
        entry->peer_protocol = local->unsi_conn_pcb;
        int error = copy_path(&local->unsi_addr.ua_sun, entry->local_path, &entry->local_length);
        if (error) return error;
        error = copy_path(&local->unsi_caddr.ua_sun, entry->peer_path, &entry->peer_length);
        if (error) return error;
    }
    return 0;
}
