#!/bin/sh
# Assembles the GitHub Pages site into _site: the page, the screenshots and the fonts
# the app bundles. Run from the repository root.
set -eu
rm -rf _site
mkdir -p _site/fonts
cp site/index.html site/style.css _site/
cp -R docs/screenshots _site/screenshots
cp fonts/inter/Inter-Regular.ttf fonts/inter/Inter-Medium.ttf fonts/inter/Inter-SemiBold.ttf \
  fonts/inter/OFL.txt _site/fonts/
cp fonts/jetbrains-mono/JetBrainsMonoNL-Regular.ttf _site/fonts/
cp fonts/jetbrains-mono/OFL.txt _site/fonts/OFL-JetBrainsMono.txt
