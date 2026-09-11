-- OS type for the machine, from the node's os_image/os.id at discovery time.
-- "talos" for Talos Linux nodes, "baremetal" for any other OS (RHEL, Ubuntu,
-- ...). Used by the SSH+kexec in-place conversion to decide which nodes are
-- convertible, and so the status reconciler can exempt non-Talos nodes from
-- the Talos API probe (which would otherwise flip them to "offline").
ALTER TABLE machines ADD COLUMN os_type TEXT;
