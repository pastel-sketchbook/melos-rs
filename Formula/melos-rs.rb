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
  version "0.7.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-rs-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "fb8bb53e8c308dc2dee54f3088c8f3bb9aad7fbaf546526117697e63414af155"
    end

    on_intel do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-rs-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "962aa4608f9473376822962bbf9ae91c59669f3f90b7ced8517ff014f75b287d"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-rs-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "73d96f7311aa6f3ec6eda12041a3cf68010d9c4c26e2173d1919e5d7074ee081"
    end

    on_intel do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-rs-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "18dac1a5ca8b238b429ea692f18e7ccd85cb20c1ff911b3f0dad132f7287c7bb"
    end
  end

  def install
    bin.install "melos-rs"
  end

  test do
    assert_match "melos-rs", shell_output("#{bin}/melos-rs --version")
  end
end
