; Highlights for tree-sitter-kotlin-ng, which doesn't ship any.
; When patterns capture the same node, the first one wins.

(line_comment) @comment
(block_comment) @comment
(shebang) @comment

(string_literal) @string
(multiline_string_literal) @string
(character_literal) @string
(escape_sequence) @string.escape
; the code inside "${...}" is code again
(interpolation) @embedded
(interpolation ["$" "${" "}"] @punctuation.special)

(number_literal) @number
(float_literal) @number
((identifier) @constant.builtin
  (#any-of? @constant.builtin "true" "false" "null"))

(annotation "@" @attribute)
(annotation (user_type (identifier) @attribute))
(annotation (constructor_invocation (user_type (identifier) @attribute)))
(label) @label

(function_declaration name: (identifier) @function)
(call_expression (identifier) @function.call)
(call_expression (navigation_expression (identifier) @function.call .))

(class_declaration name: (identifier) @type)
(object_declaration name: (identifier) @type)
(type_alias type: (identifier) @type)
(user_type (identifier) @type)
(enum_entry (identifier) @constant)

(parameter (identifier) @variable.parameter)
(navigation_expression (identifier) @property .)

(this_expression) @variable.builtin
(super_expression) @variable.builtin

[
  "package" "import" "class" "interface" "fun" "object" "val" "var" "typealias"
  "constructor" "by" "companion" "init" "this" "super"
  "if" "else" "when" "try" "catch" "finally" "for" "do" "while" "throw" "return"
  "as" "as?" "is" "!is" "in" "!in" "out" "where"
  "get" "set" "field" "file" "property" "receiver" "param" "setparam" "delegate" "dynamic"
  "abstract" "final" "open" "sealed" "data" "enum" "inner" "annotation" "value"
  "inline" "infix" "operator" "suspend" "tailrec" "external" "override" "lateinit"
  "const" "vararg" "noinline" "crossinline"
  "public" "private" "protected" "internal" "expect" "actual"
  "return@" "this@" "super@"
] @keyword
