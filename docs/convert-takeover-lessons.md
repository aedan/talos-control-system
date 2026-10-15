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
| `kexec` not installed | `command not found` even with `PATH=/usr/sbin` | Fail the node before kexec and tell the operator to install `kexec-tools` (`load_kexec` false). Do not treat a missing binary as a PATH-only problem. |
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

## Retry (fresh kubespray)

Lab was wiped after convert destroyed kube namespaces. Next convert should:

1. Select **control planes first** (or include at least one CP) so etcd snapshot + identity harvest happen before workers.
2. If a worker canary is required, use the **same cluster** convert job later so the machine CA is reused (`prior_identity_from_jobs`).
3. Set `controlPlaneEndpoint` to the kubeconfig API server (not 127.0.0.1, not a CP that 401s bootstrap tokens).
4. Pick factory modules (`bnx2-bnx2x`, `iscsi-tools`, `nfs-utils` as needed).
5. Do not start a second convert while one is running.

## Run #2 (2026-10) — keep the live etcd

Phobos was a healthy Genestack cluster (Kube-OVN, Ceph, OpenStack) on Ubuntu. The convert has to land Talos without wiping data disks or throwing away etcd. These are the gaps run #2 hit, and what the scheduler does about them now.

| What happened | Effect | Prevent in TCS |
|---|---|---|
| Pod CIDR hardcoded `10.244.0.0/16` | Live pod CIDR was `10.236.0.0/14` | Harvest `--cluster-cidr` from kube-controller-manager. Fall back to 10.244.0.0/16 only when harvest is empty. |
| Nameserver `169.254.25.10` (NodeLocal DNS) kept, NTP left at `time.cloudflare.com` | Talos refuses bootstrap until time syncs; DNS lookup of the public NTP name times out | Drop `127.*` and `169.254.*` nameservers on capture and again before kexec. Harvest chrony/timesyncd servers into `machine.time.servers`. |
| No `cluster.etcd.advertisedSubnets` or `machine.kubelet.nodeIP.validSubnets` | Kube-OVN join `100.64.0.0/16` became the node InternalIP and the etcd peer URL. A later member cached that peer and crash-looped | Set both to the node's infrastructure subnet (the captured address masked to its prefix) before etcd or kubelet start. |
| Bootstrap token group `system:bootstrappers:nodes` | Kubespray only binds `system:bootstrappers:kubeadm:default-node-token`. Kubelet CSR create is forbidden | Inject that kubeadm group, and apply ClusterRoleBindings for it and for `system:bootstrappers:nodes`. |
| First CP marked done when `bootstrap --recover-from` returned | The snapshot from job start was stale. The next controller was installed against the wrong etcd | Record the snapshot revision. Before bootstrap, if another Ubuntu controller has a higher revision, snapshot that member and use it. After bootstrap, require `talosctl etcd status` and a Talos node object. Do not bootstrap again. `AlreadyExists` / data dir not empty fails the node; empty only the EPHEMERAL volume on the system disk. |
| `controlPlane.endpoint` stayed on the kubeconfig server | That apiserver died with the old etcd quorum, so the next controller could not join | After the recovered member is healthy, set the endpoint to that node. If a later install targets the endpoint host, retarget to a control plane already done. |
| Kube-OVN hostPath `/etc/origin/openvswitch` | Talos `/etc` is read-only. A cluster-wide DaemonSet edit rolls every Ubuntu node | Do not edit the live DaemonSet. Label the converted node `tcs.local/os=talos`. Operators run a separate Talos DaemonSet (hostPath `/var/lib/openvswitch`, `OnDelete`, node affinity on that label) and exclude the label from the Ubuntu DaemonSet. Same `/etc` constraint applies to ovn-central, libvirt, and iSCSI config. |
| Factory image kubelet newer than the harvested apiserver | v1.13.11 ships kubelet v1.36 while this cluster's apiserver is v1.35.4. Skew is unsupported | The job still uses the operator-selected Talos version. Pick a release whose kubelet matches the harvested apiserver. Do not reinstall nodes that are already Ready solely to undo skew. |
| Worker-only job minted a new machine CA | The new workers cannot join control planes that already run the previous CA | A worker-only job reads `talosctl get mc` from the live control plane (cluster talosconfig, or the previous job's admin cert) and copies that machine CA, cluster id, secret, and tokens. It does not mint. The explicit `controlPlaneEndpoint` replaces the endpoint stored in that config. |
| Node marked done when `osImage` contained Talos | The node was still NotReady while Kube-OVN pulled its image. The next worker started against a broken CNI | Label `tcs.local/os=talos` as soon as the OS image is Talos, then wait until the Ready condition is True. A Talos NotReady node stays in `join` for about an hour and is failed, not done. The job status is failed if any node failed. Adopt only nodes whose state is `done`. |
| `apply-config -m reboot` returned while `:50000` stayed open | The reboot finished before the next tick, so the probe never saw the installer leave and failed a node that was already Talos and Ready. The Talos label was never set, so the Ubuntu Kube-OVN pods stayed on the node | If the cluster talosconfig can call `version`, or the node object is already Talos, treat that as the installed OS and wait for Ready. Do not send a second reboot while the installer is still up. |
| Maintenance reboot while the installer was still writing the disk | The machine booted the previous Ubuntu system. Five workers (`worker04`, `worker11`, `worker14`, `worker16`, `worker42`) were Ready on Ubuntu again after "apid never came back" | Do not call `talosctl reboot` on the maintenance API. `kexec -e` runs only after a successful `kexec -l`. If ssh returns and `:50000` does not, retry the install once. If the node stays off the network, wait longer, then fail. A chassis power cycle is an operator action, not a scheduler step. |
| kexec into the installer on an `ixgbe` bond (`931314-ceph04`, slaves `eno1`/`eno2`/`eno3`) | `kexec -e` left the node dark. A GRUB reboot did start Talos. The kernel renamed those ixgbe ports to `eno1`/`eno2`/`eno3`, so the Ubuntu names were already right. `bond=bond0:eno1,eno2,eno3:mode=802.3ad` left every slave as backup: the switch sends no LACP, and without `miimon` the bond never marks a slave active, so `:50000` never answered. Adding `xmit_hash_policy=layer2,miimon=100,lacp_rate=fast,ad_select=stable` brought an active slave up and `:50000` answered. The kernel still does not parse `bond=`; Talos userspace does. Firmware tries UEFI PXE (`grubx64.efi` from `172.20.0.126`) and that TFTP fetch fails. | Boot the installer from GRUB. For 802.3ad, put `miimon=100`, `lacp_rate=fast`, `xmit_hash_policy=layer2`, and `ad_select=stable` on the `bond=` cmdline and the same fields in the machine config. Copy `reset_devices`, `panic=10`, `bond=`, and `ip=` into `install.extraKernelArgs`; the installer GRUB does not inherit the boot append. Set the BMC boot device to disk before the install reboot. Install only the captured root disk. |
| Ready wait of 30 minutes expired | `ceph02`, `worker22`, and `worker23` were marked failed and were kubectl Ready on Talos later | Wait about an hour. A worker that is already kubectl Ready on Talos is marked done and is not reinstalled. |
| Reused identity had empty `ntpServers` | Workers fell back to `time.cloudflare.com` | Copy `machine.time.servers` and `cluster.network.podSubnets` from the live control plane when those fields are empty on the stored identity. |

Success for a controller is still kubectl Ready with the original CNI, on the restored etcd, with peer URLs on the infrastructure subnet. `:50000`, a Talos `osImage`, and a bootstrap exit are not success. A worker is done only when kubectl Ready is True.

`install.wipe` stays limited to the captured root disk. Do not `talosctl reset` STATE or user disks. Do not power-cycle as a recovery step.
