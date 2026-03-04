# PowerShell build script for Cassie Graph DB API
# Quick development commands

param(
    [Parameter(Position = 0)]
    [string]$Command = "help"
)

function Write-Step {
    param([string]$Message)
    Write-Host $Message -ForegroundColor Cyan
}

function Write-Success {
    param([string]$Message)
    Write-Host $Message -ForegroundColor Green
}

function Write-ErrorMsg {
    param([string]$Message)
    Write-Host $Message -ForegroundColor Red
}

function Invoke-Help {
    Write-Host "Cassie Graph DB Development Commands:" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "  help          " -ForegroundColor Cyan -NoNewline
    Write-Host "Show this help message"
    Write-Host "  check         " -ForegroundColor Cyan -NoNewline
    Write-Host "Run all checks (format, clippy, unit tests)"
    Write-Host "  fmt           " -ForegroundColor Cyan -NoNewline
    Write-Host "Format code with rustfmt"
    Write-Host "  clippy        " -ForegroundColor Cyan -NoNewline
    Write-Host "Run clippy linter"
    Write-Host "  test          " -ForegroundColor Cyan -NoNewline
    Write-Host "Run unit tests only (no DB required)"
    Write-Host "  integration   " -ForegroundColor Cyan -NoNewline
    Write-Host "Run integration tests (requires Cassandra)"
    Write-Host "  pre-commit    " -ForegroundColor Cyan -NoNewline
    Write-Host "Simulate CI checks exactly"
    Write-Host "  db-up         " -ForegroundColor Cyan -NoNewline
    Write-Host "Start Cassandra in Docker"
    Write-Host "  db-down       " -ForegroundColor Cyan -NoNewline
    Write-Host "Stop Cassandra"
    Write-Host "  db-status     " -ForegroundColor Cyan -NoNewline
    Write-Host "Check Cassandra health"
    Write-Host "  run           " -ForegroundColor Cyan -NoNewline
    Write-Host "Run the API server (requires Cassandra)"
    Write-Host "  build         " -ForegroundColor Cyan -NoNewline
    Write-Host "Build release binary"
    Write-Host "  clean         " -ForegroundColor Cyan -NoNewline
    Write-Host "Clean build artifacts"
    Write-Host ""
    Write-Host "Usage: .\make.ps1 <command>" -ForegroundColor Gray
}

function Invoke-Check {
    Write-Step "[*] Running all checks..."
    Invoke-Fmt
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Invoke-Clippy
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Invoke-Test
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Success "[+] All checks passed!"
}

function Invoke-Fmt {
    Write-Step "[*] Formatting code..."
    cargo fmt --all
    if ($LASTEXITCODE -eq 0) {
        Write-Success "[+] Code formatted"
    }
    else {
        Write-ErrorMsg "[-] Formatting failed"
        exit $LASTEXITCODE
    }
}

function Invoke-Clippy {
    Write-Step "[*] Running clippy..."
    cargo clippy --all-targets --all-features -- -D warnings
    if ($LASTEXITCODE -eq 0) {
        Write-Success "[+] Clippy passed"
    }
    else {
        Write-ErrorMsg "[-] Clippy failed"
        exit $LASTEXITCODE
    }
}

function Invoke-Test {
    Write-Step "[*] Running unit tests (no DB required)..."
    cargo test --lib -- --nocapture
    if ($LASTEXITCODE -eq 0) {
        Write-Success "[+] Unit tests passed"
    }
    else {
        Write-ErrorMsg "[-] Unit tests failed"
        exit $LASTEXITCODE
    }
}

function Invoke-Integration {
    Write-Step "[*] Checking Cassandra is healthy..."
    $status = docker inspect cassie-db --format="{{.State.Health.Status}}" 2>$null
    if ($status -ne "healthy") {
        Write-ErrorMsg "[-] Cassandra is not healthy (status: $status). Run: .\make.ps1 db-up"
        exit 1
    }
    Write-Step "[*] Running integration tests..."
    cargo test --test integration -- --nocapture
    if ($LASTEXITCODE -eq 0) {
        Write-Success "[+] Integration tests passed"
    }
    else {
        Write-ErrorMsg "[-] Integration tests failed"
        exit $LASTEXITCODE
    }
}

function Invoke-PreCommit {
    Write-Step "[*] Simulating CI checks..."

    Write-Step "[1/3] Format check..."
    cargo fmt -- --check
    if ($LASTEXITCODE -ne 0) {
        Write-ErrorMsg "[-] Format check failed. Run: .\make.ps1 fmt"
        exit $LASTEXITCODE
    }

    Write-Step "[2/3] Clippy..."
    cargo clippy --all-targets --all-features -- -D warnings
    if ($LASTEXITCODE -ne 0) {
        Write-ErrorMsg "[-] Clippy failed"
        exit $LASTEXITCODE
    }

    Write-Step "[3/3] Unit tests..."
    cargo test --lib -- --nocapture
    if ($LASTEXITCODE -ne 0) {
        Write-ErrorMsg "[-] Tests failed"
        exit $LASTEXITCODE
    }

    Write-Success "[+] All CI checks passed! Safe to push."
}

function Invoke-DbUp {
    Write-Step "[*] Starting Cassandra..."
    docker compose up -d
    if ($LASTEXITCODE -ne 0) {
        Write-ErrorMsg "[-] Failed to start Cassandra"
        exit $LASTEXITCODE
    }
    Write-Step "[*] Waiting for Cassandra to be healthy (this takes ~45s)..."
    $attempts = 0
    do {
        Start-Sleep -Seconds 5
        $attempts++
        $status = docker inspect cassie-db --format="{{.State.Health.Status}}" 2>$null
        Write-Host "  [$($attempts * 5)s] $status"
    } while ($status -ne "healthy" -and $attempts -lt 18)

    if ($status -eq "healthy") {
        Write-Success "[+] Cassandra is ready"
    }
    else {
        Write-ErrorMsg "[-] Cassandra did not become healthy in time"
        exit 1
    }
}

function Invoke-DbDown {
    Write-Step "[*] Stopping Cassandra..."
    docker compose down
    if ($LASTEXITCODE -eq 0) {
        Write-Success "[+] Cassandra stopped"
    }
    else {
        Write-ErrorMsg "[-] Failed to stop Cassandra"
        exit $LASTEXITCODE
    }
}

function Invoke-DbStatus {
    $status = docker inspect cassie-db --format="{{.State.Health.Status}}" 2>$null
    if ($null -eq $status -or $status -eq "") {
        Write-Host "Cassandra: " -NoNewline
        Write-Host "not running" -ForegroundColor Red
    }
    elseif ($status -eq "healthy") {
        Write-Host "Cassandra: " -NoNewline
        Write-Host "healthy" -ForegroundColor Green
    }
    else {
        Write-Host "Cassandra: " -NoNewline
        Write-Host $status -ForegroundColor Yellow
    }
}

function Invoke-Run {
    Write-Step "[*] Starting cassie-api..."
    $env:CASSANDRA_HOST = "127.0.0.1:9042"
    $env:SERVER_PORT = "8080"
    $env:RUST_LOG = "cassie_api=debug"
    cargo run --bin cassie-api
}

function Invoke-Build {
    Write-Step "[*] Building release binary..."
    cargo build --release --bin cassie-api
    if ($LASTEXITCODE -eq 0) {
        Write-Success "[+] Binary: target/release/cassie-api"
    }
    else {
        Write-ErrorMsg "[-] Build failed"
        exit $LASTEXITCODE
    }
}

function Invoke-Clean {
    Write-Step "[*] Cleaning build artifacts..."
    cargo clean
    if ($LASTEXITCODE -eq 0) {
        Write-Success "[+] Clean complete"
    }
    else {
        Write-ErrorMsg "[-] Clean failed"
        exit $LASTEXITCODE
    }
}

# Main
switch ($Command.ToLower()) {
    "help" { Invoke-Help }
    "check" { Invoke-Check }
    "fmt" { Invoke-Fmt }
    "clippy" { Invoke-Clippy }
    "test" { Invoke-Test }
    "integration" { Invoke-Integration }
    "pre-commit" { Invoke-PreCommit }
    "db-up" { Invoke-DbUp }
    "db-down" { Invoke-DbDown }
    "db-status" { Invoke-DbStatus }
    "run" { Invoke-Run }
    "build" { Invoke-Build }
    "clean" { Invoke-Clean }
    default {
        Write-Host "Unknown command: $Command" -ForegroundColor Red
        Write-Host ""
        Invoke-Help
        exit 1
    }
}
