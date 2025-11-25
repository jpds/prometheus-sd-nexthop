{ packages, ... }:

{
  name = "prometheus-sd-nexthop";

  nodes = {
    router =
      { pkgs, lib, ... }:
      {
        networking.firewall.allowedTCPPorts = [ 9198 ];

        services.prometheus.exporters.blackbox = {
          enable = true;
          openFirewall = true;
          configFile = pkgs.writeText "config.yml" (
            builtins.toJSON {
              modules.icmp_v6 = {
                prober = "icmp";
                icmp.preferred_ip_protocol = "ip6";
              };
            }
          );
        };

        systemd.services.prometheus-sd-nexthop = {
          wantedBy = [ "multi-user.target" ];
          after = [ "network.target" ];
          serviceConfig = {
            ExecStart = "${lib.getExe packages.prometheus-sd-nexthop}";
            User = "prometheus-sd-nexthop";
            Group = "prometheus-sd-nexthop";
            Restart = "on-failure";
            DynamicUser = true;
            CapabilityBoundingSet = [ "" ];
            DevicePolicy = "closed";
            LockPersonality = true;
            MemoryDenyWriteExecute = true;
            NoNewPrivileges = true;
            PrivateDevices = true;
            ProcSubset = "pid";
            ProtectClock = true;
            ProtectHome = true;
            ProtectHostname = true;
            ProtectControlGroups = true;
            ProtectKernelLogs = true;
            ProtectKernelModules = true;
            ProtectKernelTunables = true;
            ProtectProc = "invisible";
            ProtectSystem = "strict";
            RestrictAddressFamilies = [
              "AF_INET"
              "AF_INET6"
              "AF_NETLINK"
            ];
            RestrictNamespaces = true;
            RestrictRealtime = true;
            RestrictSUIDSGID = true;
            SystemCallArchitectures = "native";
            SystemCallFilter = [
              # 1. allow a reasonable set of syscalls
              "@system-service @resources"
              # 2. and deny unreasonable ones
              "~@privileged"
              # 3. then allow the required subset within denied groups
              "@chown"
            ];
          };
        };
      };

    prometheus =
      { pkgs, lib, ... }:
      {
        environment.systemPackages = [
          pkgs.jq
        ];

        services.prometheus = {
          enable = true;
          scrapeConfigs = [
            {
              job_name = "prometheus";
              static_configs = [
                {
                  targets = [
                    "localhost:9090"
                  ];
                }
              ];
            }
            {
              job_name = "prometheus-sd-nexthop";
              static_configs = [
                {
                  targets = [
                    "router:9198"
                  ];
                }
              ];
            }
            {
              job_name = "blackbox-router-nexthop";
              metrics_path = "/probe";
              params = {
                module = [ "icmp" ];
              };
              http_sd_configs = [
                {
                  url = "http://router:9198/";
                }
              ];
              relabel_configs = [
                {
                  source_labels = [ "__address__" ];
                  target_label = "__param_target";
                }
                {
                  source_labels = [ "__param_target" ];
                  target_label = "instance";
                }
                {
                  target_label = "__address__";
                  replacement = "router:9115";
                }
              ];
            }
          ];
        };
      };
  };

  testScript = ''
    start_all()

    prometheus.wait_for_unit("prometheus")
    prometheus.wait_for_open_port(9090)

    router.wait_for_unit("prometheus-blackbox-exporter")
    router.wait_for_open_port(9115)
    router.wait_for_unit("prometheus-sd-nexthop")
    router.wait_for_open_port(9198)

    router.wait_until_succeeds(
      "journalctl -o cat -u prometheus-sd-nexthop.service | grep 'Starting prometheus-sd-nexthop server'"
    )

    router.systemctl("start network-online.target")
    router.wait_for_unit("network-online.target")

    prometheus.wait_until_succeeds(
      "curl -sf 'http://127.0.0.1:9090/api/v1/query?query=sum(axum_http_requests_total)' | "
      + "jq '.data.result[0].value[1]' | grep -v '\"0\"'"
    )

    prometheus.wait_until_succeeds(
      "curl -sf 'http://127.0.0.1:9090/api/v1/query?query=prometheus_sd_discovered_targets\{config=\"blackbox-router-nexthop\"\}' | "
      + "jq '.data.result[0].value[1]' | grep '\"2\"'"
    )

    # This should be ==1 for UP, but the integration testing framework blocks
    # ICMP so just check that the count is valid for DOWN
    prometheus.wait_until_succeeds(
      "curl -sf 'http://127.0.0.1:9090/api/v1/query?query=count(up\{job=\"blackbox-router-nexthop\"\}==0)' | "
      + "jq '.data.result[0].value[1]' | grep '\"2\"'"
    )

    router.log(router.succeed("systemd-analyze security prometheus-sd-nexthop.service | grep -v '✓'"))
  '';
}
