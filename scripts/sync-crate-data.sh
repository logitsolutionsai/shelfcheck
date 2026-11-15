#!/usr/bin/env sh
# Copy the data files each library crate embeds into that crate, so
# `cargo package` and crates.io get them. The copies at the workspace root
# (rules/, assets/fonts/) are the ones to edit; run this script after any
# change there. The test `crate_data_matches_workspace` (shelfcheck-core and
# shelfcheck-report) fails while a copy differs.
# Usage: scripts/sync-crate-data.sh   (from the workspace root)
set -eu
rm -rf crates/shelfcheck-core/rules crates/shelfcheck-report/assets/fonts
mkdir -p crates/shelfcheck-core crates/shelfcheck-report/assets
cp -R rules crates/shelfcheck-core/rules
cp -R assets/fonts crates/shelfcheck-report/assets/fonts
for crate in shelfcheck-core shelfcheck-report shelfcheck-cli; do
  cp LICENSE NOTICE "crates/$crate/"
done
echo "Copied rules/ into shelfcheck-core, assets/fonts/ into shelfcheck-report, LICENSE and NOTICE into every crate."
