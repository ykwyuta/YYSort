* 12-byte input. Keep the key, mark the end, and copy the key after the payload.
 INCLUDE COND=(1,4,CH,EQ,1,4,CH)
 INREC OVERLAY=(11:C'OK',13:1,4)
 SORT FIELDS=(13,4,CH,A)
 OUTREC BUILD=(1,12,14:C'END',X)
