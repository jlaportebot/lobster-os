#!/usr/bin/env python3
"""Generate MicroPython qstrdefs.generated.h from source files.
Scans all .c and .h files for MP_QSTR_xxx references and generates
the complete qstr definition header."""
import os, re, sys

def main():
    src_dir = sys.argv[1]
    out_dir = sys.argv[2]
    
    # Find all MP_QSTR_xxx references
    qstrs = set()
    for root, dirs, files in os.walk(src_dir):
        for fname in files:
            if fname.endswith('.c') or fname.endswith('.h'):
                fpath = os.path.join(root, fname)
                with open(fpath, errors='ignore') as f:
                    content = f.read()
                for m in re.findall(r'MP_QSTR_(\w+)', content):
                    qstrs.add(m)
    
    # Add base entries from qstrdefs.h
    base = ['*', '_', '/', ' ', '\\n', '<module>', '<lambda>',
            '<listcomp>', '<dictcomp>', '<setcomp>', '<genexpr>',
            '<string>', '<stdin>', 'utf-8',
            '%#o', '%#x', '{:#o}', '{:#x}', '{:#b}']
    
    # Write preprocessed input for makeqstrdata.py
    pp_path = os.path.join(out_dir, 'qstrdefs.preprocessed.h')
    with open(pp_path, 'w') as f:
        f.write('QCFG(BYTES_IN_LEN, 1)\n')
        f.write('QCFG(BYTES_IN_HASH, 1)\n')
        for entry in base:
            f.write(f'Q({entry})\n')
        for qstr in sorted(qstrs):
            f.write(f'Q({qstr})\n')
    
    # Run makeqstrdata.py to generate final header
    import subprocess
    makeqstrdata = os.path.join(src_dir, 'py', 'makeqstrdata.py')
    result = subprocess.run(
        ['python3', makeqstrdata, pp_path],
        capture_output=True, text=True, timeout=30
    )
    
    if result.returncode == 0:
        out_path = os.path.join(out_dir, 'qstrdefs.generated.h')
        with open(out_path, 'w') as f:
            f.write(result.stdout)
        print(f"Generated {out_path}: {os.path.getsize(out_path)} bytes, {len(qstrs)} QSTRs")
        
        # Generate root_pointers.h — only scan py/ and our port, not all port dirs
        import re as re2
        root_pointers = set()
        # Only scan py/ directory (core, no hardware deps)
        for fn2 in os.listdir(os.path.join(src_dir, 'py')):
            if fn2.endswith('.c'):
                with open(os.path.join(src_dir, 'py', fn2), errors='ignore') as f2obj:
                    c2 = f2obj.read()
                for m2 in re2.findall(r'MP_REGISTER_ROOT_POINTER\((.*?)\);', c2):
                    root_pointers.add(m2)
        # Also scan shared/runtime (core runtime files)
        for fn2 in os.listdir(os.path.join(src_dir, 'shared', 'runtime')):
            if fn2.endswith('.c'):
                with open(os.path.join(src_dir, 'shared', 'runtime', fn2), errors='ignore') as f2obj:
                    c2 = f2obj.read()
                for m2 in re2.findall(r'MP_REGISTER_ROOT_POINTER\((.*?)\);', c2):
                    root_pointers.add(m2)
        # Also scan shared/readline
        rl_dir = os.path.join(src_dir, 'shared', 'readline')
        if os.path.isdir(rl_dir):
            for fn2 in os.listdir(rl_dir):
                if fn2.endswith('.c'):
                    with open(os.path.join(rl_dir, fn2), errors='ignore') as f2obj:
                        c2 = f2obj.read()
                    for m2 in re2.findall(r'MP_REGISTER_ROOT_POINTER\((.*?)\);', c2):
                        root_pointers.add(m2)
        rp_path = os.path.join(out_dir, 'root_pointers.h')
        with open(rp_path, 'w') as f:
            f.write("// Automatically generated.\n\n")
            for item in sorted(root_pointers):
                f.write(item + ";\n")
        
        # Generate moduledefs.h — only scan py/ core
        moduledefs = {}
        # Files to skip (have MICROPY_PY_THREAD gates that require thread support)
        skip_files_mods = {'modthread.c'}  # thread module requires GIL/thread support
        for fn3 in os.listdir(os.path.join(src_dir, 'py')):
            if fn3.endswith('.c') and fn3 not in skip_files_mods:
                with open(os.path.join(src_dir, 'py', fn3), errors='ignore') as f3obj:
                    c3 = f3obj.read()
                for m3 in re2.findall(r'MP_REGISTER_MODULE\((\w+),\s*(\w+)(?:,\s*(\w+))?\)', c3):
                    mod_qstr = m3[0]
                    mod_obj = m3[1]
                    moduledefs[f"MP_MODULE_{mod_qstr}"] = (mod_qstr, mod_obj)
        # Also scan shared/runtime
        for fn3 in os.listdir(os.path.join(src_dir, 'shared', 'runtime')):
            if fn3.endswith('.c'):
                with open(os.path.join(src_dir, 'shared', 'runtime', fn3), errors='ignore') as f3obj:
                    c3 = f3obj.read()
                for m3 in re2.findall(r'MP_REGISTER_MODULE\((\w+),\s*(\w+)(?:,\s*(\w+))?\)', c3):
                    moduledefs[f"MP_MODULE_{m3[0]}"] = (m3[0], m3[1])
        md_path = os.path.join(out_dir, 'moduledefs.h')
        with open(md_path, 'w') as f:
            f.write("// Automatically generated moduledefs.h\n\n")
            f.write("extern const char mp_frozen_mpy_names[];\n\n")
            for mod_def_name, (mod_qstr, mod_obj) in sorted(moduledefs.items()):
                f.write(f"extern const struct _mp_obj_module_t {mod_obj};\n")
                f.write(f"#undef {mod_def_name}\n")
                f.write(f"#define {mod_def_name} {{ MP_ROM_QSTR({mod_qstr}), MP_ROM_PTR(&{mod_obj}) }},\n")
            f.write("\n#define MICROPY_REGISTERED_MODULES \\\n")
            for mod_def_name in sorted(moduledefs.keys()):
                f.write(f"    {mod_def_name} \\\n")
            f.write("// MICROPY_REGISTERED_MODULES\n")
            f.write("\n#define MICROPY_HAVE_REGISTERED_EXTENSIBLE_MODULES 0\n")
        
        # Generate mpversion.h
        with open(os.path.join(out_dir, 'mpversion.h'), 'w') as f:
            f.write("// Auto-generated version\n")
            f.write('#define MICROPY_VERSION_MAJOR 1\n')
            f.write('#define MICROPY_VERSION_MINOR 25\n')
            f.write('#define MICROPY_VERSION_MICRO 0\n')
            f.write('#define MICROPY_VERSION_STRING "1.25.0"\n')
            f.write('#define MICROPY_GIT_TAG "LobsterOS-1.25.0"\n')
            f.write('#define MICROPY_BUILD_DATE "2026-08-16"\n')
        
        # Create stub headers
        stubs = ['compressed.data.h', 'modstm_const.h', 'modstm_mpz.h',
                  'pins.h', 'pins_af.h', 'pins_af_const.h', 'pins_af_defs.h',
                  'pins_board.h', 'pllfreqtable.h', 'plli2stable.h', 'pybcdc_inf.h']
        for s in stubs:
            with open(os.path.join(out_dir, s), 'w') as f:
                f.write(f"// Stub: {s}\n")
    else:
        # Fallback: just write the preprocessed file as the generated header
        # This works if we also define the Q() macro appropriately
        import shutil
        out_path = os.path.join(out_dir, 'qstrdefs.generated.h')
        shutil.copy(pp_path, out_path)
        print(f"Fallback: copied preprocessed to {out_path} ({os.path.getsize(out_path)} bytes)")
        print(f"makeqstrdata.py error: {result.stderr[:200]}")

if __name__ == '__main__':
    main()
