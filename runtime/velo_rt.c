#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <time.h>
#include <unistd.h>
#endif

static uint64_t velo_rng_state = 0;

void velo_rt_print_i64(int64_t v) {
    printf("%lld", (long long)v);
}

void velo_rt_print_u64(uint64_t v) {
    printf("%llu", (unsigned long long)v);
}

void velo_rt_print_f64(double v) {
    printf("%g", v);
}

void velo_rt_print_bool(int8_t v) {
    fputs(v ? "true" : "false", stdout);
}

void velo_rt_print_str(const uint8_t* ptr, int64_t len) {
    if (ptr && len > 0) {
        fwrite(ptr, 1, (size_t)len, stdout);
    }
}

void velo_rt_print_cstr(const char* s) {
    if (s) {
        fputs(s, stdout);
    }
}

void velo_rt_print_newline(void) {
    putchar('\n');
    fflush(stdout);
}

void velo_rt_print_space(void) {
    putchar(' ');
}

void velo_rt_panic(const uint8_t* ptr, int64_t len, const uint8_t* file_ptr, int64_t file_len, int64_t line) {
    fputs("velo panic at ", stderr);
    if (file_ptr && file_len > 0) {
        fwrite(file_ptr, 1, (size_t)file_len, stderr);
    } else {
        fputs("<unknown>", stderr);
    }
    fprintf(stderr, ":%lld: ", (long long)line);
    if (ptr && len > 0) {
        fwrite(ptr, 1, (size_t)len, stderr);
    }
    fputc('\n', stderr);
    fflush(stderr);
    exit(1);
}

void velo_rt_bounds_fail(int64_t idx, int64_t len, const uint8_t* file_ptr, int64_t file_len, int64_t line) {
    fputs("velo index out of bounds at ", stderr);
    if (file_ptr && file_len > 0) {
        fwrite(file_ptr, 1, (size_t)file_len, stderr);
    } else {
        fputs("<unknown>", stderr);
    }
    fprintf(stderr, ":%lld: index %lld out of bounds for length %lld\n",
            (long long)line, (long long)idx, (long long)len);
    fflush(stderr);
    exit(1);
}

void* velo_rt_alloc(int64_t bytes) {
    if (bytes <= 0) {
        bytes = 1;
    }
    void* p = calloc(1, (size_t)bytes);
    if (!p) {
        fputs("velo fatal: out of memory\n", stderr);
        exit(1);
    }
    return p;
}

void* velo_rt_realloc(void* p, int64_t bytes) {
    if (bytes <= 0) {
        bytes = 1;
    }
    void* np = realloc(p, (size_t)bytes);
    if (!np) {
        fputs("velo fatal: out of memory in realloc\n", stderr);
        exit(1);
    }
    return np;
}

void velo_rt_free(void* p) {
    free(p);
}

void velo_rt_copy_mem(void* dst, const void* src, int64_t bytes) {
    if (dst && src && bytes > 0) {
        memcpy(dst, src, (size_t)bytes);
    }
}

void velo_rt_set_mem(void* dst, uint8_t val, int64_t bytes) {
    if (dst && bytes > 0) {
        memset(dst, val, (size_t)bytes);
    }
}

int64_t velo_rt_time_ns(void) {
#ifdef _WIN32
    static LARGE_INTEGER freq = {0};
    if (freq.QuadPart == 0) {
        QueryPerformanceFrequency(&freq);
    }
    LARGE_INTEGER now;
    QueryPerformanceCounter(&now);
    /* Split into seconds + remainder to avoid int64 overflow */
    int64_t secs = now.QuadPart / freq.QuadPart;
    int64_t rem = now.QuadPart % freq.QuadPart;
    return secs * 1000000000LL + (rem * 1000000000LL) / freq.QuadPart;
#else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000000000LL + (int64_t)ts.tv_nsec;
#endif
}

void velo_rt_sleep_ms(int32_t ms) {
    if (ms <= 0) return;
#ifdef _WIN32
    Sleep((DWORD)ms);
#else
    usleep((useconds_t)ms * 1000);
#endif
}

uint64_t velo_rt_rand_u64(void) {
    if (velo_rng_state == 0) {
        uint64_t seed = (uint64_t)velo_rt_time_ns() ^ 0x9E3779B97F4A7C15ULL;
        velo_rng_state = seed ? seed : 0x85EBCA6BULL;
    }
    uint64_t x = velo_rng_state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    velo_rng_state = x;
    return x * 0x2545F4914F6CDD1DULL;
}

int64_t velo_rt_rand_range(int64_t min_v, int64_t max_v) {
    if (max_v <= min_v) return min_v;
    uint64_t span = (uint64_t)(max_v - min_v);
    return min_v + (int64_t)(velo_rt_rand_u64() % span);
}

int64_t velo_rt_read_int(void) {
    long long val = 0;
    if (scanf("%lld", &val) == 1) {
        int ch;
        while ((ch = getchar()) != '\n' && ch != EOF) {}
        return (int64_t)val;
    }
    int ch;
    while ((ch = getchar()) != '\n' && ch != EOF) {}
    return 0;
}

int64_t velo_rt_read_line(uint8_t** out_ptr) {
    size_t cap = 128;
    size_t len = 0;
    uint8_t* buf = (uint8_t*)velo_rt_alloc((int64_t)cap);
    int ch;
    while ((ch = getchar()) != EOF && ch != '\n') {
        if (ch == '\r') continue;
        if (len + 2 >= cap) {
            cap *= 2;
            buf = (uint8_t*)velo_rt_realloc(buf, (int64_t)cap);
        }
        buf[len++] = (uint8_t)ch;
    }
    buf[len] = '\0';
    *out_ptr = buf;
    return (int64_t)len;
}

void velo_rt_str_concat(const uint8_t* a_ptr, int64_t a_len,
                        const uint8_t* b_ptr, int64_t b_len,
                        uint8_t** out_ptr, int64_t* out_len) {
    if (a_len < 0) a_len = 0;
    if (b_len < 0) b_len = 0;
    int64_t total = a_len + b_len;
    uint8_t* buf = (uint8_t*)velo_rt_alloc(total + 1);
    if (a_ptr && a_len > 0) memcpy(buf, a_ptr, (size_t)a_len);
    if (b_ptr && b_len > 0) memcpy(buf + a_len, b_ptr, (size_t)b_len);
    buf[total] = '\0';
    *out_ptr = buf;
    *out_len = total;
}

void velo_rt_i64_to_str(int64_t v, uint8_t** out_ptr, int64_t* out_len) {
    char tmp[32];
    int n = snprintf(tmp, sizeof(tmp), "%lld", (long long)v);
    if (n < 0) n = 0;
    uint8_t* buf = (uint8_t*)velo_rt_alloc(n + 1);
    memcpy(buf, tmp, (size_t)n);
    buf[n] = '\0';
    *out_ptr = buf;
    *out_len = (int64_t)n;
}

void velo_rt_u64_to_str(uint64_t v, uint8_t** out_ptr, int64_t* out_len) {
    char tmp[32];
    int n = snprintf(tmp, sizeof(tmp), "%llu", (unsigned long long)v);
    if (n < 0) n = 0;
    uint8_t* buf = (uint8_t*)velo_rt_alloc(n + 1);
    memcpy(buf, tmp, (size_t)n);
    buf[n] = '\0';
    *out_ptr = buf;
    *out_len = (int64_t)n;
}

void velo_rt_f64_to_str(double v, int32_t prec, uint8_t** out_ptr, int64_t* out_len) {
    char tmp[64];
    int n;
    if (prec >= 0 && prec <= 16) {
        n = snprintf(tmp, sizeof(tmp), "%.*f", (int)prec, v);
    } else {
        n = snprintf(tmp, sizeof(tmp), "%g", v);
    }
    if (n < 0) n = 0;
    uint8_t* buf = (uint8_t*)velo_rt_alloc(n + 1);
    memcpy(buf, tmp, (size_t)n);
    buf[n] = '\0';
    *out_ptr = buf;
    *out_len = (int64_t)n;
}

void velo_rt_bool_to_str(int8_t v, uint8_t** out_ptr, int64_t* out_len) {
    const char* s = v ? "true" : "false";
    int64_t n = v ? 4 : 5;
    uint8_t* buf = (uint8_t*)velo_rt_alloc(n + 1);
    memcpy(buf, s, (size_t)(n + 1));
    *out_ptr = buf;
    *out_len = n;
}

int8_t velo_rt_str_eq(const uint8_t* a_ptr, int64_t a_len,
                      const uint8_t* b_ptr, int64_t b_len) {
    if (a_len != b_len) return 0;
    if (a_len == 0) return 1;
    if (a_ptr == b_ptr) return 1;
    if (!a_ptr || !b_ptr) return 0;
    return memcmp(a_ptr, b_ptr, (size_t)a_len) == 0 ? 1 : 0;
}

int32_t velo_rt_str_cmp(const uint8_t* a_ptr, int64_t a_len,
                        const uint8_t* b_ptr, int64_t b_len) {
    int64_t min_len = a_len < b_len ? a_len : b_len;
    if (min_len > 0 && a_ptr && b_ptr) {
        int c = memcmp(a_ptr, b_ptr, (size_t)min_len);
        if (c != 0) return c < 0 ? -1 : 1;
    }
    if (a_len < b_len) return -1;
    if (a_len > b_len) return 1;
    return 0;
}

int64_t velo_rt_cstr_len(const char* s) {
    return s ? (int64_t)strlen(s) : 0;
}

double velo_rt_sqrt(double x) { return sqrt(x); }
double velo_rt_sin(double x) { return sin(x); }
double velo_rt_cos(double x) { return cos(x); }
double velo_rt_tan(double x) { return tan(x); }
double velo_rt_floor(double x) { return floor(x); }
double velo_rt_ceil(double x) { return ceil(x); }
double velo_rt_round(double x) { return round(x); }
double velo_rt_pow(double x, double y) { return pow(x, y); }
