#include <signal.h>
#include <lobsteros/syscalls.h>

int sigreturn(unsigned long n);

struct ksa {
	void *handler;
	unsigned long flags;
	int (*restorer)(unsigned long n);
	sigset_t mask;
};

int __sigaction(int sig, const struct sigaction *sa, struct sigaction *old)
{
	struct ksa ksa, _ksa;
	if (sa) {
		ksa.handler = sa->sa_handler;
		ksa.flags = sa->sa_flags | SA_RESTORER;
		ksa.restorer = sigreturn;
		ksa.mask = sa->sa_mask;
	}
	if (syscall4(SYS_SIGACTION, sig, (long)(sa ? &ksa : 0), (long)(old ? &_ksa : 0), 8))
		return -1;
	if (old) {
		old->sa_handler = _ksa.handler;
		old->sa_flags = _ksa.flags;
		old->sa_mask = _ksa.mask;
	}
	return 0;
}

sighandler_t signal(int sig, sighandler_t func)
{
	struct sigaction sa = {.sa_handler = func, .sa_flags = SA_RESTART};
	if (__sigaction(sig, &sa, &sa) < 0)
		return SIG_ERR;
	return sa.sa_handler;
}
