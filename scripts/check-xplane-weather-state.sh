#!/usr/bin/env bash
# Test the X-Plane weather transaction without an X-Plane installation.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
weather_source="${repo_root}/sim/xplane/weather"
weather_test_root="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-weather-state.XXXXXX")"
trap 'rm -rf "${weather_test_root}"' EXIT

weather_compiler="${CXX:-c++}"
"${weather_compiler}" -std=c++17 -O2 -Wall -Wextra -Werror \
  -I "${weather_source}" \
  "${weather_source}/weather_state_tests.cpp" \
  "${weather_source}/weather_state.cpp" \
  -o "${weather_test_root}/weather_state_tests"
"${weather_test_root}/weather_state_tests"

fake_xplm="${weather_source}/test_support/xplm"
"${weather_compiler}" -std=c++17 -O2 -Wall -Wextra -Werror \
  -I "${fake_xplm}" -I "${weather_source}" \
  "${weather_source}/weather_plugin_tests.cpp" \
  "${weather_source}/PilotageWeather.cpp" \
  "${weather_source}/weather_state.cpp" \
  "${fake_xplm}/fake_xplm.cpp" \
  -o "${weather_test_root}/weather_plugin_tests"
"${weather_test_root}/weather_plugin_tests"

echo "check-xplane-weather-state: OK"
