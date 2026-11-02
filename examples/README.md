---
title: front matter is hidden
---
# Amoxtli Sample

Welcome to **Amoxtli**, a *native* Markdown viewer. It supports `inline code`, ~~strikethrough~~,
[external links](https://gnome.org), [links to sections](#tables), and [other files](notes/guide.md).
Line one with a hard break  
line two. Footnotes work as well.[^1]

## Lists

- First item
- Second item with a fairly long line of text that should wrap neatly under the start of the text and not the bullet marker
  - Nested item
    - Deeper item
- [x] Finished task
- [ ] Pending task

1. One
2. Two
3. Three

## Quotes

> Amoxtli means *book* in Nahuatl.
>
> > Nested quotes work too.

> [!TIP]
> GitHub-style alerts are supported too.

> [!WARNING]
> Careful with this one.

## Code

```python
def greet(name: str) -> str:
    """Say hello."""
    return f"Hello, {name}!"  # comment
```

    indented code block

## Tables

| Feature | Status | Notes |
|:--------|:------:|------:|
| Headings | ✅ | **bold** |
| Tables | ✅ | `code` |
| Images | ✅ | [link](https://example.com) |

---

### Image

![Amoxtli icon](../data/icons/hicolor/scalable/apps/io.github.amoxtli.Amoxtli.svg)

#### Level four
##### Level five
###### Level six

[^1]: This is the footnote text, with a [link back](#amoxtli-sample).
