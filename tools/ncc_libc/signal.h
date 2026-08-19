#define NSIG		32

#define SIGHUP		 1
#define SIGINT		 2
#define SIGQUIT		 3
#define SIGILL		 4
#define SIGTRAP		 5
#define SIGABRT		 6
#define SIGIOT		 6
#define SIGFPE		 8
#define SIGKILL		 9
#define SIGSEGV		11
#define SIGPIPE		13
#define SIGALRM		14
#define SIGTERM		15
#define SIGUNUSED	31
#define SIGBUS		 7
#define SIGUSR1		10
#define SIGUSR2		12
#define SIGSTKFLT	16
#define SIGCHLD		17
#define SIGCONT		18
#define SIGSTOP		19
#define SIGTSTP		20
#define SIGTTIN		21
#define SIGTTOU		22
#define SIGURG		23
#define SIGXCPU		24
#define SIGXFSZ		25
#define SIGVTALRM	26
#define SIGPROF		27
#define SIGWINCH	28
#define SIGIO		29
#define SIGPWR		30
#define SIGSYS		31

#define SIGCLD		SIGCHLD
#define SIGPOLL		SIGIO

#define SIG_BLOCK	0
#define SIG_UNBLOCK	1
#define SIG_SETMASK	2

#define SA_NOCLDSTOP	1
#define SA_NOCLDWAIT	2
#define SA_SIGINFO	4
#define SA_ONSTACK	0x08000000
#define SA_RESTART	0x10000000
#define SA_NODEFER	0x40000000
#define SA_RESETHAND	0x80000000
#define SA_RESTORER	0x04000000

struct sigset_t {
	unsigned long __bits[128 / sizeof(long)];
};
typedef struct sigset_t sigset_t;

struct sigaction {
	void (*sa_handler)(int);
	sigset_t sa_mask;
	int sa_flags;
	void (*sa_restorer)(void);
};

typedef void (*sighandler_t)(int);

#define SIG_ERR		((void (*)(int)) -1)
#define SIG_DFL		((void (*)(int)) 0)
#define SIG_IGN		((void (*)(int)) 1)
#define SIG_HOLD	((void (*)(int)) 2)

int sigaction(int signum, const struct sigaction *act, struct sigaction *oldact);
int sigemptyset(sigset_t *set);
int sigfillset(sigset_t *set);
int sigaddset(sigset_t *set, int signum);
int sigdelset(sigset_t *set, int signum);
int sigismember(const sigset_t *set, int signum);
int sigprocmask(int how, const sigset_t *set, sigset_t *oldset);
sighandler_t signal(int signum, sighandler_t action);
int kill(int pid, int sig);
int raise(int sig);
