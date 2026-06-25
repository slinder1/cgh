// Copyright © 2026 Advanced Micro Devices, Inc. All rights reserved.
// SPDX-License-Identifier: MIT

mod cgh;
mod change;
mod cli;
mod env;
mod gh;
mod metadata;
mod util;

fn main() -> anyhow::Result<()> {
    cgh::cgh()
}
