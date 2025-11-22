{ packages, ... }:

{
  name = "prometheus-sd-nexthop";

  nodes = {
    machine =
      { pkgs, lib, ... }:
      {
        systemd.services.prometheus-sd-nexthop = {
          wantedBy = [ "multi-user.target" ];
          after = [ "network.target" ];
          serviceConfig = {
            ExecStart = "${lib.getExe packages.prometheus-sd-nexthop}";
            User = "prometheus-sd-nexthop";
            Group = "prometheus-sd-nexthop";
            Restart = "on-failure";
            DynamicUser = true;
          };
        };
      };
  };

  testScript = ''
    machine.wait_for_unit("prometheus-sd-nexthop")
    machine.wait_for_open_port(9198)

    machine.wait_until_succeeds(
      "journalctl -o cat -u prometheus-sd-nexthop.service | grep 'Starting prometheus-sd-nexthop server'"
    )

    machine.systemctl("start network-online.target")
    machine.wait_for_unit("network-online.target")

    import json

    target_json = json.loads(machine.wait_until_succeeds("curl http://localhost:9198/"))

    assert len(target_json[0]['targets']) == 2
  '';
}
