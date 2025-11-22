# Prometheus SD Nexthop

`prometheus-sd-nexthop` is a tool that dynamically takes a Linux host's gateway
IP addresses and presents them to Prometheus's HTTP service discovery
functionality.

It is intended to be used alongside `blackbox_exporter`[^1] on a Linux-based
router to monitor the availability of the equipment belonging to a user's ISP.

## Example blackbox_exporter configuration

Configure Prometheus as follows to have the `router-nexthop` job dynamically
get IP addresses to probe from this service discovery tool (which listens on
port `9198` by default) - and probe them with the `blackbox_exporter` prober: 

```yaml
- job_name: prometheus-sd-nexthop
  targets:
  - 10.0.0.1:9198
- job_name: router-nexthop
  metrics_path: /probe
  params:
    module: [icmp]
  scrape_interval: 20s
  relabel_configs:
  - source_labels: [__address__]
    target_label: __param_target
  - source_labels: [__param_target]
    target_label: instance
  - target_label: __address__
    replacement: 10.0.0.1:9115 # blackbox_exporter's port
  http_sd_configs:
  - url: http://10.0.0.1:9198/ # This service discovery service
```

This tool also exposes its own metrics at the `/metrics` endpoint.

[^1]: https://github.com/prometheus/blackbox_exporter
