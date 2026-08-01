	processor 16F15376
	psect isavectors,class=CODE,delta=2
	global _isa_vectors
_isa_vectors:
	addfsr 0, 1
	addlw 1
	addwf 0x70, w
	addwfc 0x70, f
	andlw 1
	andwf 0x70, w
	asrf 0x70, f
	bcf 0x70, 0
	bra $+1
	brw
	bsf 0x70, 0
	btfsc 0x70, 0
	btfss 0x70, 0
	call 0x123
	callw
	clrf 0x70
	clrw
	clrwdt
	comf 0x70, f
	decf 0x70, f
	decfsz 0x70, f
	goto 0x123
	incf 0x70, f
	incfsz 0x70, f
	iorlw 1
	iorwf 0x70, f
	lslf 0x70, f
	lsrf 0x70, f
	movf 0x70, w
	moviw ++fsr0
	movlb 1
	movlp 1
	movlw 1
	movwf 0x70
	movwi fsr0++
	nop
	reset
	retfie
	retlw 1
	return
	rlf 0x70, f
	rrf 0x70, f
	sleep
	sublw 1
	subwf 0x70, f
	subwfb 0x70, f
	swapf 0x70, f
	xorlw 1
	xorwf 0x70, f
	; Addressing-form coverage used to derive strict decode masks.
	moviw ++fsr1
	moviw --fsr0
	moviw --fsr1
	moviw fsr0++
	moviw fsr1++
	moviw fsr0--
	moviw fsr1--
	moviw 1[fsr0]
	moviw -1[fsr1]
	movwi ++fsr0
	movwi ++fsr1
	movwi --fsr0
	movwi --fsr1
	movwi fsr1++
	movwi fsr0--
	movwi fsr1--
	movwi 1[fsr0]
	movwi -1[fsr1]
	addfsr 1, -1
