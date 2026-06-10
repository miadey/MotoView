/*
 * shim.c — intentionally (almost) empty.
 *
 * SwiftPM requires a clang target to have at least one compilable source file.
 * The real symbols live in the Rust static archive (libmotoview_client.a) that
 * the MotoViewKit target links via linkerSettings; this file only gives the
 * MotoViewFFI clang module a translation unit so its public header
 * (motoview_ffi.h) is importable from Swift as `import MotoViewFFI`.
 */
#include "motoview_ffi.h"

/* A no-op so the TU is non-empty and the symbol table has something local. */
void mv_ffi_link_anchor(void) {}
