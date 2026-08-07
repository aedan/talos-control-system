# Kubernetes Deployment Guide

This guide covers deploying TCS on a Kubernetes cluster with production-grade configuration.

## Prerequisites

- Kubernetes cluster (1.28+)
- Helm 3.12+
- External database (PostgreSQL recommended for production)
- TLS certificates (cert-manager recommended)
- Ingress controller (nginx, traefik, or similar)

## Quick Install

```bash
helm repo add tcs https://charts.talos.dev
helm install tcs tcs/tcs -n tcs --create-namespace
```

## Production Configuration

### 1. Create a values file

```yaml
# values-prod.yaml

replicaCount: 2

image:
  repository: ghcr.io/siderolabs/talos-control-system
  tag: "0.1.0"
  pullPolicy: IfNotPresent

service:
  type: ClusterIP
  port: 8081
  grpcPort: 8080
  metricsPort: 9090

ingress:
  enabled: true
  className: nginx
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
    nginx.ingress.kubernetes.io/proxy-body-size: "50m"
  hosts:
    - host: tcs.example.com
      paths:
        - path: /
          pathType: Prefix
  tls:
    - secretName: tcs-tls
      hosts:
        - tcs.example.com

persistence:
  enabled: true
  size: 20Gi
  storageClass: standard

database:
  backend: postgres
  postgresUrl: "postgresql://tcs:CHANGE_PASSWORD@postgres-primary:5432/tcs"
  maxConnections: 20
  connectionTimeout: 30

siderolink:
  bindPort: 8082
  listenPort: 443
  mtu: 1420
  subnet: "100.64.0.0/10"

resources:
  requests:
    cpu: 250m
    memory: 256Mi
  limits:
    cpu: 2000m
    memory: 4Gi

branding:
  name: "Acme Kubernetes Platform"
  shortName: "Acme K8s"
  tagline: "Managed by Acme Corp"
  primaryColor: "#2563EB"
  secondaryColor: "#60A5FA"
  backgroundColor: "#0F172A"
  surfaceColor: "#1E293B"
  textColor: "#F8FAFC"
  textMutedColor: "#94A3B8"
  docsUrl: "https://docs.acme.example.com"
  supportUrl: "https://support.acme.example.com"
```

### 2. Install with production values

```bash
helm install tcs tcs/tcs -n tcs --create-namespace -f values-prod.yaml
```

## Exposing Siderolink

Machines need to reach TCS on the siderolink port (default 8082). Options:

### Option A: NodePort

```yaml
service:
  type: NodePort
siderolink:
  bindPort: 8082
  listenPort: 30082
```

### Option B: LoadBalancer

```yaml
siderolink:
  service:
    type: LoadBalancer
    port: 443
```

### Option C: Second Ingress (WebSocket Upgrade)

```yaml
# Add to values.yaml
ingress:
  hosts:
    - host: tcs.example.com
      paths:
        - path: /
          pathType: Prefix
    - host: link.tcs.example.com
      paths:
        - path: /
          pathType: Prefix

ingressAnnotations:
  nginx.ingress.kubernetes.io/use-regex: "true"
  nginx.ingress.kubernetes.io/proxy-read-timeout: "3600"
  nginx.ingress.kubernetes.io/proxy-send-timeout: "3600"
```

## High Availability

### Multiple Replicas

```yaml
replicaCount: 3

# With PostgreSQL, multiple replicas are safe since the database
# handles state. With SQLite, use replicaCount: 1.

affinity:
  podAntiAffinity:
    preferredDuringSchedulingIgnoredDuringExecution:
      - weight: 100
        podAffinityTerm:
          labelSelector:
            matchExpressions:
              - key: app.kubernetes.io/name
                operator: In
                values:
                  - tcs
          topologyKey: kubernetes.io/hostname
```

### PostgreSQL Primary/Replica

For a production PostgreSQL backend, deploy a managed database or use a Helm chart like `postgresql` or `cloudnative-pg`:

```yaml
# Using cloudnative-pg
helm install cnpg-postgres cloudnative-pg/cloudnative-pg -n cnpg --create-namespace

# Then configure TCS:
database:
  backend: postgres
  postgresUrl: "postgresql://tcs:password@cnpg-postgres-rw:5432/tcs"
```

## Upgrading

```bash
helm repo update
helm upgrade tcs tcs/tcs -n tcs -f values-prod.yaml
```

TCS runs database migrations automatically on startup. If upgrading across major versions, review the changelog for breaking changes.

### Rollback

```bash
helm rollback tcs 1 -n tcs
```

## Resource Recommendations

| Deployment Size | CPU Request | Memory Request | CPU Limit | Memory Limit |
|----------------|-------------|----------------|-----------|--------------|
| Small (< 5 clusters) | 100m | 128Mi | 500m | 1Gi |
| Medium (5-20 clusters) | 250m | 256Mi | 2000m | 4Gi |
| Large (20+ clusters) | 500m | 512Mi | 4000m | 8Gi |

## Monitoring

TCS exposes Prometheus metrics on port 9090:

```yaml
# Prometheus ServiceMonitor
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: tcs
  namespace: tcs
spec:
  selector:
    matchLabels:
      app.kubernetes.io/name: tcs
  endpoints:
    - port: metrics
      interval: 15s
```

Key metrics:
- `tcs_clusters_total` — Total managed clusters
- `tcs_machines_total` — Total registered machines
- `tcs_machines_by_status` — Machine count by status
- `tcs_http_requests_total` — API request counter
- `tcs_http_request_duration_seconds` — Request latency histogram
- `tcs_siderolink_connections_active` — Active siderolink tunnels

## Backup and Restore

### Database Backup

```bash
# SQLite
kubectl exec -n tcs tcs-tcs-0 -- cp /var/lib/tcs/data.db /tmp/data.db
kubectl cp -n tcs tcs-tcs-0:/tmp/data.db ./backup.db

# PostgreSQL
pg_dump -h postgres-host -U tcs tcs > backup.sql
```

### Restore

```bash
# Stop TCS pods
kubectl scale deployment tcs-tcs -n tcs --replicas=0

# Restore database (SQLite example)
kubectl cp ./backup.db -n tcs tcs-tcs-0:/var/lib/tcs/data.db

# Scale back up
kubectl scale deployment tcs-tcs -n tcs --replicas=1
```

## Troubleshooting

### Check pod status

```bash
kubectl get pods -n tcs
kubectl describe pod -n tcs -l app.kubernetes.io/name=tcs
kubectl logs -n tcs deployment/tcs-tcs
```

### Verify siderolink connectivity

```bash
kubectl port-forward -n tcs svc/tcs-tcs 8082:8082

# From a Talos machine:
nc -zv $(minikube ip) 8082
```

### Check ingress

```bash
kubectl get ingress -n tcs
kubectl get pods -n ingress-nginx
kubectl logs -n ingress-nginx -l app.kubernetes.io/name=ingress-nginx
```
