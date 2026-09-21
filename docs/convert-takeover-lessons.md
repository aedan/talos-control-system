# Convert / takeover lessons (Phobos 2026-09)

What broke the live Kubespray→Talos overtake, how it was recovered, and what TCS
must do so the next convert does not need this rescue. Lab notes live in
`HANDOFF-phobos-recover.md` (gitignored). This file is the product checklist.

Cluster identity that recover joined: infra02 talosconfig (machine CA from the
**control-plane** convert job, not the earlier worker-only fleet job).

## Convert must never do these

| What happened | Effect | Prevent in TCS |
|---|---|---|
| Worker machineconfig carried `machine.ca` **crt+key** (CP spec with `type: worker`) | machined rejects config; kernel bond comes up; **ping, all TCP closed** | Workers only `machine.acceptedCAs` (crt). CPs only get issuing key. Already in `build_install_config`. |
| STATE rewritten at hardcoded `/dev/sda5` | v1.13 layout is often EFI/META/STATE/EPHEMERAL (`sda3`=STATE). On 6-part boxes `sda5` **is** STATE; writing a bad YAML + sysrq still bricks userspace. Ubuntu later cannot even mount Talos XFS (`incompat 0xc0`). | Never edit STATE by number. Convert uses kexec/installer `apply-config`. Recover uses BMC installer or `apply-config --insecure`. |
| Two convert jobs (fleet workers vs CP) | Two machine CAs. Fleet workers' apid will not speak the CP talosconfig. | One identity for the cluster. Persist talosconfig on adopt. Re-apply workers with CP identity if a split already happened. |
| Convert “done” = TCP `:50000` | Workers without trustd often never listen on apid; kubelet+OS-IMAGE Talos is success. | Wait kubectl OS Talos (already) **and** `talosctl version` with cluster CA when talosconfig exists. |
| kubeadm bootstrap token unpadded / CSR Approved-not-Issued | Join hangs | 6.16 token + openssl-sign kubelet client CSR with harvested k8s CA (already). |
| Stock kexec image missing `bnx2-bnx2x` | 10Gb NICs dead after kexec | Factory schematic; operator-selected modules (already). |
| `kexec` not on PATH | `command not found` | Prefix `PATH=/usr/sbin:/sbin`. |
| `kexec_file_load` + factory `initramfs-*.xz` | File is **zstd** (`28 b5 2f fd`). Kernel `ip=` comes up, initrd dropped, ping-only. | Decompress to raw cpio; `kexec -c` (legacy load) then fallback `-l` (`a1ef21a`, `a1fc3e3`). |
| kexec from **MAAS ephemeral** Ubuntu | Even with initrd loaded, userspace still dies | Convert kexec from the **disk** Ubuntu OS (kubespray node), not from rescue ramdisk. Rescue is a disk editor only. |
| CP kube-apiserver endpoint `127.0.0.1` or `.55` 401 | Bootstrap fails | `kubeconfigServer` from live kubeconfig (`.38` here) (already). |
| Host CNI files on workers | Bricked canaries | Do not inject host CNI conflist on convert. |
| iLO4 SOL `console=ttyS0` at 9600 | Silent SOL | `extraKernelArgs`: `ttyS0,115200` and `ttyS1,115200`. |
| iLO4 virtual CD URL | `image_inserted=yes`, BMC never HTTP GETs ISO | Do not depend on iLO URL mount. PXE (if TCS owns DHCP) or kexec from disk OS. |
| BMC password via machine PATCH | Stored unencrypted | `PUT /machines/:id/bmc` only. |
| Status reconciler probes Talos with no talosconfig | Whole fleet flaps offline | Skip Talos probe without talosconfig (already). |
| Recover `install.wipe` on a node that already has apid | Unnecessary reinstall | `bmc_recover` + apid up → `configure` without wipe (`a1fc3e3`). |
| After installer, next boot still PXE | MAAS `netboot=true` loops; `power cycle` can leave chassis **off** | `set_boot(Disk)` after apply; **power on** if off (`69eca5b`). |
| Rescue after `netboot=false` | BMC `bootdev pxe` but MAAS DHCP still localboots disk — **no rescue SSH** | Set `netboot=true` before rescue PXE; set false only after STATE is written. |
| Parallel MAAS rescue | "Entering rescue mode" times out at **30 min**; abort/release storms | Serialize rescue; `machine abort` first; wait SSH up to 25 min. |
| Worker YAML cloned from a CP spec | Leftover `eno3`/`enp3s0f*`; wrong bond slaves (`eno49` vs `eno1`) | Render **only** captured `node.network` (`render_node_network_yaml`). |
| Convert HA lock 300s | Jobs cannot cancel after restart | Document: stop TCS and DELETE `ha_locks`. |

## Recover paths that actually worked (this lab)

1. **Apid up, wrong CA** — `talosctl apply-config --insecure -f worker.yaml` (captured bond/IP, acceptedCAs, infra02 identity). worker08/16.
2. **Kubelet up, no apid (Talos)** — privileged pod, mount **`/dev/disk/by-label/STATE` or by-partlabel**, write worker.yaml, reboot. worker15 (sda3), 47/49 (sda5).
3. **Ping-only (kernel up, no userspace)** — MAAS rescue as **disk editor** (do not kexec): if STATE unmountable, grow STATE to ≥400MiB (Ubuntu mkfs.xfs refuses 100MiB), mkfs, write `config.yaml`, then `netboot=false`, BMC disk, **power on**. worker35 first; then 20+ nodes.
4. **STATE already written, chassis off** — disk boot + power on only. Seven nodes in one shot.

Product TCS should prefer (1) then BMC installer+apply (not MAAS rescue, which is lab-specific). (2) is a k8s-privileged-pod trick; do not put that in TCS.

## Convert success criteria

A node is converted when **all** of:

- `kubectl` Ready **and** `OS-IMAGE` contains Talos (or worker-only: kubelet + OS Talos if apid never appears).
- `talosctl --talosconfig <cluster> -n <ip> version` shows Server (when talosconfig exists).
- TCS machine status `running` for Talos, not merely `:50000` open.

## Still open on Phobos (takeover in progress)

- infra01 still kubeadm API — convert last.
- Ubuntu leftovers worker06/09/11 (kubelet, no SSH) and worker32 (SSH).
- Rescue SSH / PXE failures on some workers (MAAS never offered PXE).
- `FAIL no apid` at 360s is often too short; many came up later.
