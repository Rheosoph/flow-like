/*
 * Compatibility shim for the prebuilt ONNX Runtime archive on the Linux release
 * baseline.
 *
 * `ort` downloads a static ONNX Runtime from pyke's CDN. Those archives are
 * compiled on a host with glibc >= 2.38 and libstdc++ >= 13, which redirects the
 * string-to-integer functions to their C23 variants and emits
 * __cxa_call_terminate on cleanup paths that may throw. Neither exists on the
 * Ubuntu 22.04 / glibc 2.35 baseline the Linux artifacts are built against, so
 * the desktop binary fails to link with undefined references. Building on a
 * newer runner is not an alternative: it would stamp GLIBC_2.38 into the
 * shipped binary and lose every distribution below it.
 *
 * The C23 functions differ from the C17 ones only in accepting a 0b prefix for
 * base 0 and base 2, which ONNX Runtime never parses, so forwarding is exact for
 * every input it produces.
 *
 * prepare-ort-glibc-compat.sh compiles this and only links it when the build
 * toolchain does not already provide these symbols. Delete both files once the
 * Linux baseline reaches glibc 2.38 and GCC 13.
 */

#define _GNU_SOURCE

#include <locale.h>
#include <stdlib.h>

long __isoc23_strtol(const char *nptr, char **endptr, int base) {
	return strtol(nptr, endptr, base);
}

unsigned long __isoc23_strtoul(const char *nptr, char **endptr, int base) {
	return strtoul(nptr, endptr, base);
}

long long __isoc23_strtoll(const char *nptr, char **endptr, int base) {
	return strtoll(nptr, endptr, base);
}

unsigned long long __isoc23_strtoull(const char *nptr, char **endptr, int base) {
	return strtoull(nptr, endptr, base);
}

long long __isoc23_strtoll_l(const char *nptr, char **endptr, int base, locale_t loc) {
	return strtoll_l(nptr, endptr, base, loc);
}

unsigned long long __isoc23_strtoull_l(const char *nptr, char **endptr, int base, locale_t loc) {
	return strtoull_l(nptr, endptr, base, loc);
}

/*
 * std::terminate() and __cxa_begin_catch live in libstdc++. RUSTFLAGS applies to
 * build script executables too, and those do not link libstdc++, so both
 * references are weak and the shim degrades to abort() when they are absent.
 * _ZSt9terminatev is the Itanium ABI mangling of std::terminate(), spelled out
 * so this stays a C translation unit.
 */
extern void _ZSt9terminatev(void) __attribute__((weak));
extern void *__cxa_begin_catch(void *unwind_exception) __attribute__((weak));

__attribute__((noreturn)) void __cxa_call_terminate(void *unwind_exception) {
	if (unwind_exception != NULL && __cxa_begin_catch != NULL) {
		__cxa_begin_catch(unwind_exception);
	}

	if (_ZSt9terminatev != NULL) {
		_ZSt9terminatev();
	}

	abort();
}
