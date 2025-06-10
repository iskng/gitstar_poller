# Deployment Guide

## Health Check Endpoints

The server provides health check endpoints for orchestration systems:

### Endpoints

- **`GET /health`** or **`GET /healthz`** - Comprehensive health check
  ```json
  {
    "status": "healthy",
    "version": "0.1.0",
    "uptime_seconds": 3600,
    "checks": {
      "database": { "status": "healthy" },
      "supervisor": { "status": "healthy" },
      "workers": { "status": "healthy" }
    },
    "stats": {
      "total_accounts_processed": 42,
      "factory_queue_depth": 5,
      "factory_active_workers": 3,
      "current_worker_count": 6,
      "cpu_usage": 23.5,
      "memory_usage_percent": 45.2
    }
  }
  ```

- **`GET /livez`** - Kubernetes liveness probe
  ```json
  { "status": "alive" }
  ```

- **`GET /readyz`** - Kubernetes readiness probe
  ```json
  { "ready": true }
  ```

### Health Status Values
- `healthy` - Everything is working normally
- `degraded` - Service is working but with reduced capacity
- `unhealthy` - Service has critical issues

## Docker Deployment

### Build the Image
```bash
docker build -t github-stars-server:latest .
```

### Run with Docker Compose
```bash
docker-compose up -d
```

### Check Health
```bash
curl http://localhost:8080/health
```

## Kubernetes Deployment

### Prerequisites
- Kubernetes cluster (1.19+)
- kubectl configured
- SurrealDB deployed in the cluster

### Deploy

1. Update the secrets in `k8s-deployment.yaml`:
   ```bash
   # Edit the file and change the database password
   vim k8s-deployment.yaml
   ```

2. Apply the manifests:
   ```bash
   kubectl apply -f k8s-deployment.yaml
   ```

3. Check deployment status:
   ```bash
   kubectl -n github-stars get pods
   kubectl -n github-stars logs -f deployment/github-stars-server
   ```

4. Port-forward to test health checks:
   ```bash
   kubectl -n github-stars port-forward deployment/github-stars-server 8080:8080
   curl http://localhost:8080/health
   ```

## Production Considerations

### Resource Requirements
- **Memory**: 512MB minimum, 2GB recommended
- **CPU**: 0.25 cores minimum, 2 cores recommended
- **Storage**: Minimal (logs only)

### Environment Variables
Required:
- `DB_URL` - SurrealDB WebSocket URL
- `DB_USER` - Database username
- `DB_PASS` - Database password

Optional:
- `DB_NAMESPACE` - Default: gitstars
- `DB_DATABASE` - Default: stars
- `DB_POOL_MAX_SIZE` - Default: 10
- `HEALTH_PORT` - Default: 8080 (set to 0 to disable)
- `RUST_LOG` - Default: warn,github_stars_server=info

### Monitoring

The health check endpoint provides metrics that can be scraped by monitoring systems:
- Total accounts processed
- Queue depth
- Active/idle workers
- CPU and memory usage
- Database connection status

### Security
- Run as non-root user (included in Dockerfile)
- Use secrets for database credentials
- Consider network policies to restrict access
- Use TLS for database connections in production

### Scaling
- Currently designed for single-instance deployment
- Horizontal scaling would require:
  - External job queue (Redis/RabbitMQ)
  - Shared state management
  - Distributed locking for job claims