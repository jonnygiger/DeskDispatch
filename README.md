# DeskDispatch

DeskDispatch is a zero-JavaScript task server for GUI automations built with Rust, Axum, SQLx, Askama, PostgreSQL, and MinIO / S3.

## Environment & Prerequisites

- **Rust** (2024 edition / latest stable)
- **Docker** & **Docker Compose**

## Local Development Setup

### 1. Start Database and Storage Services

Start PostgreSQL (port 5432) and MinIO (port 9000/9001):

```bash
docker compose up -d
```

This starts:
- PostgreSQL on `localhost:5432` (database: `deskdispatch`, user: `postgres`, password: `postgrespassword`)
- MinIO S3 storage on `localhost:9000` (console on `http://localhost:9001`)
- Automatic creation of the `deskdispatch-bucket` MinIO bucket via `minio-create-bucket` service.

### 2. Environment Variables

Create a `.env` file or export environment variables:

```env
DATABASE_URL=postgres://postgres:postgrespassword@localhost:5432/deskdispatch
S3_ENDPOINT=http://localhost:9000
S3_BUCKET=deskdispatch-bucket
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadminpassword
S3_REGION=us-east-1
SESSION_SECRET=super-secret-key-change-me
BIND_ADDRESS=0.0.0.0:3000
APP_ENV=development
```

### 3. Run Migrations

Database migrations run automatically on server startup. You can also inspect migration files in `migrations/`.

### 4. Run the Server

Start the Axum server:

```bash
cargo run
```

### 5. Health Check

Verify the server is running and connected to PostgreSQL:

```bash
curl http://localhost:3000/healthz
# Returns 200 OK with "OK"
```

## Running Tests

Execute cargo tests:

```bash
cargo test
```
