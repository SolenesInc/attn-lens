# Third-party notices

Glance's original code is licensed under [MIT](LICENSE). Third-party code and
assets retain their own licenses; the project license does not replace them.

- **GPUI**, by Zed Industries, is licensed under Apache-2.0.
  `src/animation.rs` also adapts GPUI's Gaussian shadow integration from
  `src/platform/mac/shaders.metal`. See [source attribution](assets/gpui/SOURCE)
  and the [Apache-2.0 license](assets/gpui/LICENSE-APACHE).
- **Lucide icons** are licensed under ISC, with some icons derived from Feather
  under MIT. See [source attribution](assets/lucide/SOURCE) and the
  [complete icon license notices](assets/lucide/LICENSE).
- **The Glance app mark** comes from Modem's `agentpaste` project. Its source
  and revision are recorded in [the icon design notes](assets/icons/DESIGN.md).
- **Roboto Regular**, by Google, is embedded as an offline annotation fallback
  font under Apache-2.0. See [source attribution](assets/fonts/SOURCE) and the
  [font license](assets/fonts/LICENSE).
- **Hex**, by Christopher Langton, is licensed under MIT. The macOS permission
  guide (`native/permission_guide.m` and `src/permission_guide.rs`) is adapted
  from Hex's `native/permission_guide.m` and `src/desktop/permission_guide.rs`
  at commit 54d1ebf (<https://github.com/anomalyco/hex>):

  > MIT License
  >
  > Copyright (c) 2026 Christopher Langton
  >
  > Permission is hereby granted, free of charge, to any person obtaining a copy
  > of this software and associated documentation files (the "Software"), to deal
  > in the Software without restriction, including without limitation the rights
  > to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
  > copies of the Software, and to permit persons to whom the Software is
  > furnished to do so, subject to the following conditions:
  >
  > The above copyright notice and this permission notice shall be included in all
  > copies or substantial portions of the Software.
  >
  > THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
  > IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
  > FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
  > AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
  > LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
  > OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
  > SOFTWARE.

The app bundle includes this notice, Glance's license, and the GPUI, Lucide and Roboto
license files. Other Rust dependencies are recorded in `Cargo.lock`; their
upstream license terms apply to distributions that include them.
