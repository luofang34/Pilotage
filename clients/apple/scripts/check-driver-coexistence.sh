#!/bin/sh
# Verify the two-host configuration for one set of supported radios.
set -eu

client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
repository_root=$(CDPATH='' cd -- "$client_root/../.." && pwd)
source_root=${AERO_LINK_SOURCE:-"$(dirname -- "$repository_root")/aero-link"}
staged_root="$client_root/.build/aero-link"
revision=$(tr -d '[:space:]' < "$client_root/AERO_LINK_REVISION")

if [ ! -d "$staged_root/platforms/apple/AeroLink.xcodeproj" ]; then
    echo "generate the Pilotage situation project before this check" >&2
    exit 2
fi

read_setting() {
    project_file=$1
    setting=$2
    identifier=$(awk -v setting="$setting" '
        index($0, setting " = ") {
            value = $0
            sub(".*" setting " = ", "", value)
            sub(";.*", "", value)
            gsub("\"", "", value)
            print value
            exit
        }
    ' "$project_file")
    if [ -z "$identifier" ]; then
        echo "cannot read $setting from $project_file" >&2
        exit 2
    fi
    printf '%s\n' "$identifier"
}

read_git_setting() {
    repository=$1
    commit=$2
    path=$3
    setting=$4
    identifier=$(git -C "$repository" show "$commit:$path" | awk -v setting="$setting" '
        index($0, setting " = ") {
            value = $0
            sub(".*" setting " = ", "", value)
            sub(";.*", "", value)
            gsub("\"", "", value)
            print value
            exit
        }
    ')
    if [ -z "$identifier" ]; then
        echo "cannot read $setting from $path at $commit" >&2
        exit 2
    fi
    printf '%s\n' "$identifier"
}

staged_project="$staged_root/platforms/apple/AeroLink.xcodeproj/project.pbxproj"
client_project="$client_root/Pilotage.xcodeproj/project.pbxproj"
harness_host=$(read_git_setting \
    "$source_root" "$revision" \
    platforms/apple/AeroLink.xcodeproj/project.pbxproj \
    AERO_LINK_HOST_BUNDLE_IDENTIFIER)
harness_driver=$(read_git_setting \
    "$source_root" "$revision" \
    platforms/apple/AeroLink.xcodeproj/project.pbxproj \
    AERO_LINK_DRIVER_BUNDLE_IDENTIFIER)
client_host=$(read_setting "$client_project" AERO_LINK_HOST_BUNDLE_IDENTIFIER)
client_driver=$(read_setting "$staged_project" AERO_LINK_DRIVER_BUNDLE_IDENTIFIER)

if [ "$harness_host" = "$client_host" ] || [ "$harness_driver" = "$client_driver" ]; then
    echo "the AeroLink harness and Pilotage must use different App ID pairs" >&2
    exit 1
fi
case "$client_driver" in
    "$client_host".*) ;;
    *)
        echo "the Pilotage driver App ID must begin with its host App ID" >&2
        exit 1
        ;;
esac

if ! git -C "$source_root" show \
    "$revision:platforms/apple/Driver/Info.plist" \
    | cmp -s - "$staged_root/platforms/apple/Driver/Info.plist"; then
    echo "the two driver copies do not have the same radio match table" >&2
    exit 1
fi
if ! cmp -s \
    "$client_root/Configuration/AeroLinkDriverDevelopment.entitlements" \
    "$staged_root/platforms/apple/Driver/AeroLinkDriver.entitlements"; then
    echo "the Pilotage driver does not use its development entitlements" >&2
    exit 1
fi
if ! grep -q 'Driver management' "$client_root/Resources/Settings.bundle/Root.plist"; then
    echo "the Pilotage Settings bundle does not expose driver management" >&2
    exit 1
fi

echo "AeroLink two-host configuration: OK"
