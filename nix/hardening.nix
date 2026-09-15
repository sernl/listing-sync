# The service protections every unit this repository exports takes; the few a
# unit relaxes are stated at that unit.
# The protections every one of the three services takes. Two are stated per
# unit instead, because they differ for a reason rather than by oversight:
# `MemoryDenyWriteExecute`, which a JIT cannot run under, and the address
# filter, which only a service with no off-box work can close.
{
  NoNewPrivileges = true;
  CapabilityBoundingSet = "";
  AmbientCapabilities = "";
  ProtectSystem = "strict";
  ProtectHome = true;
  PrivateTmp = true;
  PrivateDevices = true;
  ProtectClock = true;
  ProtectHostname = true;
  ProtectKernelTunables = true;
  ProtectKernelModules = true;
  ProtectKernelLogs = true;
  ProtectControlGroups = true;
  ProtectProc = "invisible";
  RestrictNamespaces = true;
  RestrictRealtime = true;
  RestrictSUIDSGID = true;
  LockPersonality = true;
  RemoveIPC = true;
  SystemCallArchitectures = "native";
  SystemCallFilter = [
    "@system-service"
    "~@privileged"
    "~@resources"
  ];
  RestrictAddressFamilies = [
    "AF_INET"
    "AF_INET6"
    "AF_UNIX"
    # glibc's resolver opens a netlink socket to enumerate the machine's own
    # addresses before it answers a lookup, so a list without this turns every
    # name resolution into a failure whose message names neither DNS nor this
    # setting. It is not an egress path — what these units may reach is
    # decided by IPAddressDeny below.
    "AF_NETLINK"
  ];
  UMask = "0077";
}
