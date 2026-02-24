# typed: false
# frozen_string_literal: true

# Homebrew formula for melos-tui
#
# Install:
#   brew tap pastel-sketchbook/melos-rs https://github.com/pastel-sketchbook/melos-rs
#   brew install melos-tui
#
# After a new release, update the version, URL tags, and sha256 values below.

class MelosTui < Formula
  desc "Terminal UI for melos-rs — Flutter/Dart monorepo management"
  homepage "https://github.com/pastel-sketchbook/melos-rs"
  version "0.6.7"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-tui-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "260c12801c9a23f9c4f5451f371dd2787008903913043d284f947d7aaceb785a"
    end

    on_intel do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-tui-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "0858c6d3d949b44e5f8c404909d4adf8661c5bf65f56e3325b723812aa774d0b"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-tui-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "7e75b1e84e2b51310aa5053f2ea179c90903dcce07e278a0c6de34bb24e266ff"
    end

    on_intel do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-tui-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "34737afbab311fbe73b05719dee75d1442ef1f16bf4e8c926e8e7a4c3f17b0cb"
    end
  end

  def install
    bin.install "melos-tui"
  end

  test do
    # melos-tui requires a terminal, so just check it exists
    assert_predicate bin/"melos-tui", :executable?
  end
end
