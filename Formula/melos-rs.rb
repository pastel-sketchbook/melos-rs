# typed: false
# frozen_string_literal: true

# Homebrew formula for melos-rs
#
# Install:
#   brew tap pastel-sketchbook/melos-rs https://github.com/pastel-sketchbook/melos-rs
#   brew install melos-rs
#
# Or directly:
#   brew install pastel-sketchbook/melos-rs/melos-rs
#
# After a new release, update the version, URL tags, and sha256 values below.
# SHA256 values are in the .sha256 files attached to each GitHub release.

class MelosRs < Formula
  desc "Fast Rust replacement for Melos — Flutter/Dart monorepo management"
  homepage "https://github.com/pastel-sketchbook/melos-rs"
  version "0.6.6"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-rs-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_ARM64_SHA256"
    end

    on_intel do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-rs-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_X86_64_SHA256"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-rs-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_LINUX_ARM64_SHA256"
    end

    on_intel do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-rs-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_LINUX_X86_64_SHA256"
    end
  end

  def install
    bin.install "melos-rs"
  end

  test do
    assert_match "melos-rs", shell_output("#{bin}/melos-rs --version")
  end
end
