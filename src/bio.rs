use ::libc;
use super::openjpeg::*;

extern "C" {

  fn opj_malloc(size: size_t) -> *mut libc::c_void;

  fn opj_free(m: *mut libc::c_void);
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct opj_bio {
  pub start: *mut OPJ_BYTE,
  pub end: *mut OPJ_BYTE,
  pub bp: *mut OPJ_BYTE,
  pub buf: OPJ_UINT32,
  pub ct: OPJ_UINT32,
}
pub type opj_bio_t = opj_bio;
/* *
Write a byte
@param bio BIO handle
@return Returns OPJ_TRUE if successful, returns OPJ_FALSE otherwise
*/
/*@}*/
/*@}*/
/*
==========================================================
   local functions
==========================================================
*/
unsafe extern "C" fn opj_bio_byteout(mut bio: *mut opj_bio_t) -> OPJ_BOOL {
  (*bio).buf = (*bio).buf << 8 as libc::c_int & 0xffff as libc::c_int as libc::c_uint;
  (*bio).ct = if (*bio).buf == 0xff00 as libc::c_int as libc::c_uint {
    7 as libc::c_int
  } else {
    8 as libc::c_int
  } as OPJ_UINT32;
  if (*bio).bp as OPJ_SIZE_T >= (*bio).end as OPJ_SIZE_T {
    return 0 as libc::c_int;
  }
  let fresh0 = (*bio).bp;
  (*bio).bp = (*bio).bp.offset(1);
  *fresh0 = ((*bio).buf >> 8 as libc::c_int) as OPJ_BYTE;
  return 1 as libc::c_int;
}
/* *
Read a byte
@param bio BIO handle
@return Returns OPJ_TRUE if successful, returns OPJ_FALSE otherwise
*/
unsafe extern "C" fn opj_bio_bytein(mut bio: *mut opj_bio_t) -> OPJ_BOOL {
  (*bio).buf = (*bio).buf << 8 as libc::c_int & 0xffff as libc::c_int as libc::c_uint;
  (*bio).ct = if (*bio).buf == 0xff00 as libc::c_int as libc::c_uint {
    7 as libc::c_int
  } else {
    8 as libc::c_int
  } as OPJ_UINT32;
  if (*bio).bp as OPJ_SIZE_T >= (*bio).end as OPJ_SIZE_T {
    return 0 as libc::c_int;
  }
  let fresh1 = (*bio).bp;
  (*bio).bp = (*bio).bp.offset(1);
  (*bio).buf |= *fresh1 as libc::c_uint;
  return 1 as libc::c_int;
}
/*
 * The copyright in this software is being made available under the 2-clauses
 * BSD License, included below. This software may be subject to other third
 * party and contributor rights, including patent rights, and no such rights
 * are granted under this license.
 *
 * Copyright (c) 2002-2014, Universite catholique de Louvain (UCL), Belgium
 * Copyright (c) 2002-2014, Professor Benoit Macq
 * Copyright (c) 2001-2003, David Janssens
 * Copyright (c) 2002-2003, Yannick Verschueren
 * Copyright (c) 2003-2007, Francois-Olivier Devaux
 * Copyright (c) 2003-2014, Antonin Descampe
 * Copyright (c) 2005, Herve Drolon, FreeImage Team
 * All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions
 * are met:
 * 1. Redistributions of source code must retain the above copyright
 *    notice, this list of conditions and the following disclaimer.
 * 2. Redistributions in binary form must reproduce the above copyright
 *    notice, this list of conditions and the following disclaimer in the
 *    documentation and/or other materials provided with the distribution.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS `AS IS'
 * AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 * IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
 * ARE DISCLAIMED.  IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
 * LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
 * CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
 * SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
 * INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
 * CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
 * ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
 * POSSIBILITY OF SUCH DAMAGE.
 */
/* * @defgroup BIO BIO - Individual bit input-output stream */
/*@{*/
/* * @name Local static functions */
/*@{*/
/* *
Write a bit
@param bio BIO handle
@param b Bit to write (0 or 1)
*/
unsafe extern "C" fn opj_bio_putbit(mut bio: *mut opj_bio_t, mut b: OPJ_UINT32) {
  if (*bio).ct == 0 as libc::c_int as libc::c_uint {
    opj_bio_byteout(bio);
    /* MSD: why not check the return value of this function ? */
  }
  (*bio).ct = (*bio).ct.wrapping_sub(1);
  (*bio).buf |= b << (*bio).ct;
}
/* *
Read a bit
@param bio BIO handle
@return Returns the read bit
*/
unsafe extern "C" fn opj_bio_getbit(mut bio: *mut opj_bio_t) -> OPJ_UINT32 {
  if (*bio).ct == 0 as libc::c_int as libc::c_uint {
    opj_bio_bytein(bio);
    /* MSD: why not check the return value of this function ? */
  }
  (*bio).ct = (*bio).ct.wrapping_sub(1);
  return (*bio).buf >> (*bio).ct & 1 as libc::c_int as libc::c_uint;
}
/*
==========================================================
   Bit Input/Output interface
==========================================================
*/
#[no_mangle]
pub unsafe extern "C" fn opj_bio_create() -> *mut opj_bio_t {
  let mut bio = opj_malloc(::std::mem::size_of::<opj_bio_t>() as libc::c_ulong) as *mut opj_bio_t; /* && (n <= 32U)*/
  return bio;
}
#[no_mangle]
pub unsafe extern "C" fn opj_bio_destroy(mut bio: *mut opj_bio_t) {
  if !bio.is_null() {
    opj_free(bio as *mut libc::c_void);
  };
}
#[no_mangle]
pub unsafe extern "C" fn opj_bio_numbytes(mut bio: *mut opj_bio_t) -> ptrdiff_t {
  return (*bio).bp.wrapping_offset_from((*bio).start) as libc::c_long;
}
#[no_mangle]
pub unsafe extern "C" fn opj_bio_init_enc(
  mut bio: *mut opj_bio_t,
  mut bp: *mut OPJ_BYTE,
  mut len: OPJ_UINT32,
) {
  (*bio).start = bp;
  (*bio).end = bp.offset(len as isize);
  (*bio).bp = bp;
  (*bio).buf = 0 as libc::c_int as OPJ_UINT32;
  (*bio).ct = 8 as libc::c_int as OPJ_UINT32;
}
#[no_mangle]
pub unsafe extern "C" fn opj_bio_init_dec(
  mut bio: *mut opj_bio_t,
  mut bp: *mut OPJ_BYTE,
  mut len: OPJ_UINT32,
) {
  (*bio).start = bp;
  (*bio).end = bp.offset(len as isize);
  (*bio).bp = bp;
  (*bio).buf = 0 as libc::c_int as OPJ_UINT32;
  (*bio).ct = 0 as libc::c_int as OPJ_UINT32;
}
#[no_mangle]
pub unsafe extern "C" fn opj_bio_write(
  mut bio: *mut opj_bio_t,
  mut v: OPJ_UINT32,
  mut n: OPJ_UINT32,
) {
  let mut i: OPJ_INT32 = 0;
  assert!(n > 0 as libc::c_uint && n <= 32 as libc::c_uint);
  i = n as OPJ_INT32 - 1 as libc::c_int;
  while i >= 0 as libc::c_int {
    opj_bio_putbit(bio, v >> i & 1 as libc::c_int as libc::c_uint);
    i -= 1
  }
}
#[no_mangle]
pub unsafe extern "C" fn opj_bio_read(mut bio: *mut opj_bio_t, mut n: OPJ_UINT32) -> OPJ_UINT32 {
  let mut i: OPJ_INT32 = 0;
  let mut v: OPJ_UINT32 = 0;
  assert!(n > 0 as libc::c_uint);
  v = 0 as libc::c_uint;
  i = n as OPJ_INT32 - 1 as libc::c_int;
  while i >= 0 as libc::c_int {
    v |= opj_bio_getbit(bio) << i;
    i -= 1
    /* can't overflow, opj_bio_getbit returns 0 or 1 */
  }
  return v;
}
#[no_mangle]
pub unsafe extern "C" fn opj_bio_flush(mut bio: *mut opj_bio_t) -> OPJ_BOOL {
  if opj_bio_byteout(bio) == 0 {
    return 0 as libc::c_int;
  }
  if (*bio).ct == 7 as libc::c_int as libc::c_uint {
    if opj_bio_byteout(bio) == 0 {
      return 0 as libc::c_int;
    }
  }
  return 1 as libc::c_int;
}
#[no_mangle]
pub unsafe extern "C" fn opj_bio_inalign(mut bio: *mut opj_bio_t) -> OPJ_BOOL {
  if (*bio).buf & 0xff as libc::c_int as libc::c_uint == 0xff as libc::c_int as libc::c_uint {
    if opj_bio_bytein(bio) == 0 {
      return 0 as libc::c_int;
    }
  }
  (*bio).ct = 0 as libc::c_int as OPJ_UINT32;
  return 1 as libc::c_int;
}
