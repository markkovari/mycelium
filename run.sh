#!/usr/bin/env bash
set -euo pipefail

echo "================================================="
echo "   Mycelium - Raspberry Pi Deployment Script     "
echo "================================================="

# 1. Check for wash
if ! command -v wash &> /dev/null; then
    echo "❌ 'wash' could not be found."
    echo "Please install wasmCloud shell (wash):"
    echo "curl -sSf https://raw.githubusercontent.com/wasmCloud/wasmCloud/main/install.sh | bash"
    exit 1
fi
echo "✅ 'wash' is installed."

# 2. Check for NATS CLI
if ! command -v nats &> /dev/null; then
    echo "❌ 'nats' CLI could not be found."
    echo "Please install nats CLI (required for setting up streams):"
    echo "https://github.com/nats-io/natscli"
    echo "Or run: brew install nats-io/nats-tools/nats"
    exit 1
fi
echo "✅ 'nats' CLI is installed."

# 3. Configure Registry Credentials (if needed)
echo "-------------------------------------------------"
echo "To pull the Mycelium OCI images from GitHub Container Registry (GHCR),"
echo "you need to authenticate using a GitHub Personal Access Token (PAT)"
echo "with the 'read:packages' scope."
echo "If the repository is public, you can skip this by pressing Enter."
echo "-------------------------------------------------"

if [ -z "${WASH_REG_USER:-}" ] || [ -z "${WASH_REG_PASSWORD:-}" ]; then
    read -p "GitHub Username (leave blank to skip): " input_user
    if [ -n "$input_user" ]; then
        read -s -p "GitHub PAT: " input_password
        echo ""
        WASH_REG_USER=$input_user
        WASH_REG_PASSWORD=$input_password
    fi
fi

if [ -n "${WASH_REG_USER:-}" ] && [ -n "${WASH_REG_PASSWORD:-}" ]; then
    echo "Configuring registry credentials for ghcr.io..."
    wash reg creds put ghcr.io --username "$WASH_REG_USER" --password "$WASH_REG_PASSWORD"
    echo "✅ Credentials configured."
else
    echo "⚠️ Skipping registry authentication."
fi

# 4. Start wasmCloud Host
echo "-------------------------------------------------"
echo "Starting wasmCloud host..."
wash up --detached
echo "⏳ Waiting for host to be ready..."
sleep 5

# 5. Initialize NATS Streams
echo "-------------------------------------------------"
echo "Initializing NATS streams..."
if [ -f "infra/init-streams.sh" ]; then
    bash infra/init-streams.sh
else
    echo "❌ 'infra/init-streams.sh' not found. Ensure you are running this script from the project root."
    exit 1
fi

# 6. Deploy the Application
echo "-------------------------------------------------"
echo "Deploying Mycelium application..."
if [ -f "wadm/release.yaml" ]; then
    wash app deploy wadm/release.yaml
    echo "✅ Application deployed successfully!"
    echo "To view status, run: wash app list"
else
    echo "❌ 'wadm/release.yaml' not found. Ensure you are running this script from the project root."
    exit 1
fi

echo "================================================="
echo "   Deployment Complete!                          "
echo "================================================="
