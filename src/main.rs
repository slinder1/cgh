// Copyright © 2026 Advanced Micro Devices, Inc. All rights reserved.
// SPDX-License-Identifier: MIT

mod change;
mod cli;
mod env;
mod gd;
mod gh;
mod util;

fn main() -> anyhow::Result<()> {
    gd::gd()
}
