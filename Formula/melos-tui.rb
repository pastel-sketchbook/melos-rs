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
  version "0.6.6"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-tui-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_ARM64_SHA256"
    end

    on_intel do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-tui-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_X86_64_SHA256"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-tui-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_LINUX_ARM64_SHA256"
    end

    on_intel do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-tui-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_LINUX_X86_64_SHA256"
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
