#ifndef REMU_STDIO_H
#define REMU_STDIO_H
typedef struct remu_file FILE;
#ifndef NULL
#define NULL ((void *)0)
#endif
#define stdout ((FILE *)0)
int printf(const char *format, ...);
int puts(const char *text);
int fflush(FILE *stream);
#endif
