#!/bin/sh

# Resolve a recorded image ID, falling back to a locally built tag. Tags are
# converted to immutable IDs before callers launch a container.
resolve_toolchain_image()
{
    recorded=$1
    local_tag=$2
    if docker image inspect "$recorded" >/dev/null 2>&1
    then
        printf '%s\n' "$recorded"
        return 0
    fi
    if docker image inspect "$local_tag" >/dev/null 2>&1
    then
        docker image inspect --format '{{.Id}}' "$local_tag"
        return 0
    fi
    echo "missing Docker image: $recorded (local fallback: $local_tag)" >&2
    echo "run scripts/bootstrap-toolchains.sh first" >&2
    return 1
}
