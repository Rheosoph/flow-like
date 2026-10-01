/* Derived from Chromium chrome/test/chromedriver/js/is_option_element_toggleable.js @154.0.8037.92, Copyright The Chromium Authors, BSD-3-Clause; modified by Rheosoph GmbH. See NOTICE. */
// Copyright 2013 The Chromium Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

function isOptionElementToggleable(option) {
  if (option.tagName.toLowerCase() != 'option')
    throw new Error('element is not an option');
  for (var parent = option.parentElement;
       parent;
       parent = parent.parentElement) {
    if (parent.tagName.toLowerCase() == 'select') {
      return parent.multiple;
    }
  }
  throw new Error('option element is not in a select');
}
