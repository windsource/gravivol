# Do not exit when variable is unbound (standard is "sh -cu")
set shell := ["sh", "-c"] 

cluster := "gravivol-cluster"
registry := "gravivol-reg"

docker_arch := if arch() == "x86_64" { 
    "amd64"
} else if arch() == "aarch64" { 
    "arm64"
} else {
    error("Unknwon host architecture")
}

# List all recipes
default:
    just --list

# Create a k3d cluster with local registry and cert-manager
create-cluster:
    #!/bin/sh -e
    k3d cluster create {{cluster}} --agents 3 --registry-create {{registry}}
    PORT=$(docker inspect {{registry}} | jq -r '.[0].HostConfig.PortBindings."5000/tcp".[0].HostPort')
    echo "Local registry available at localhost:$PORT"
    kubectl apply -f tools/cert-manager.yaml

# Delete cluster and registry
delete-cluster:
  k3d cluster delete {{cluster}}

# Run integration test
itest:
    #!/bin/sh -e
    mkdir -p temp
    if [ ! -e temp/cert.pem ] || [ ! -e temp/key.pem ]; then
        openssl req -x509 -newkey rsa:4096 -nodes -keyout temp/key.pem -out temp/cert.pem -days 365 -subj '/CN=localhost'
    fi
    export GRAVIVOL_TLS_CERT_PATH=$PWD/temp/cert.pem
    export GRAVIVOL_TLS_KEY_PATH=$PWD/temp/key.pem
    cargo build
    cargo run &
    PID=$!
    sleep 1
    curl -k -H "Content-Type: application/json" -X POST --data-binary '@tools/admission-review-request.json' https://localhost:8080/mutate
    # TODO: Check result/output of curl
    kill $PID

# Build container and push to local registry
build-container:
    #!/bin/sh -e
    PORT=$(docker inspect {{registry}} | jq -r '.[0].HostConfig.PortBindings."5000/tcp".[0].HostPort')
    IMAGE=localhost:${PORT}/gravivol:main
    docker build -t $IMAGE -f tools/Dockerfile.local .
    docker push $IMAGE
