# Writes the DOCX fixtures with Microsoft Word itself, through COM: the
# documents a reader meets are the ones Word writes - style ids in the
# interface's language, runs split by revision ids, text boxes stored twice,
# fields with codes and results - and a hand-made file would have none of it.
#
# Run on Windows with Word installed, from anywhere:
#   pwsh -File make_docx.ps1
# The documents land in tests/formats, beside this folder. Every word in
# them is synthetic.
#
# Word signs what it writes with its user's name; the name is set to
# "Fixture" while the documents are written and given back afterwards, and
# the documents are saved without personal information.
$ErrorActionPreference = 'Stop'
$Out = Split-Path -Parent $PSScriptRoot

# Word's own constants, by value: the type library is not loaded here.
$wdStyleNormal = -1
$wdStyleHeading1 = -2
$wdStyleHeading2 = -3
$wdStyleTitle = -63
$wdFormatXMLDocument = 12
$wdFormatDocument97 = 0
$wdHeaderFooterPrimary = 1
$wdOutlineNumberGallery = 3
$msoTextOrientationHorizontal = 1
$wdCollapseEnd = 0

$word = New-Object -ComObject Word.Application
$word.Visible = $false
$word.DisplayAlerts = 0
$user = $word.UserName, $word.UserInitials, $word.UserAddress
$word.UserName = 'Fixture'
$word.UserInitials = 'F'
$word.UserAddress = ''

# Append a paragraph of a style at the end of the document; returns it.
function Add-Paragraph($doc, [string] $text, [int] $style = $wdStyleNormal) {
    $end = $doc.Content
    $end.Collapse($wdCollapseEnd)
    $paragraph = $doc.Paragraphs($doc.Paragraphs.Count)
    $paragraph.Range.Text = $text
    $paragraph = $doc.Paragraphs($doc.Paragraphs.Count)
    $paragraph.Style = $style
    $doc.Content.InsertParagraphAfter()
    $doc.Paragraphs($doc.Paragraphs.Count - 1)
}

# Document properties are a late-bound collection: reached by name only
# through IDispatch.
function Set-Property($doc, [string] $name, [string] $value) {
    $binding = [System.Reflection.BindingFlags]
    $property = [System.__ComObject].InvokeMember('Item', $binding::GetProperty, $null, $doc.BuiltInDocumentProperties, @($name))
    [System.__ComObject].InvokeMember('Value', $binding::SetProperty, $null, $property, @($value)) | Out-Null
}

function New-Lease($numbered) {
    $doc = $word.Documents.Add()
    Set-Property $doc 'Title' 'Office lease, Harbour Street'
    Set-Property $doc 'Author' 'Fixture'
    Set-Property $doc 'Company' ''

    Add-Paragraph $doc 'Office lease' $wdStyleTitle | Out-Null
    # The contents go here, as Word builds them: a field whose code is not
    # text, and whose result repeats the headings.
    $toc = Add-Paragraph $doc 'contents'
    Add-Paragraph $doc 'Terms' $wdStyleHeading1 | Out-Null
    Add-Paragraph $doc 'The tenant rents the second floor of the warehouse on Harbour Street for storing ceramic tiles and grout.' | Out-Null
    Add-Paragraph $doc 'Rent' $wdStyleHeading2 | Out-Null
    $rent = Add-Paragraph $doc 'The rent is 1250 euros, paid monthly by bank transfer.'
    Add-Paragraph $doc 'Rooms' $wdStyleHeading2 | Out-Null
    Add-Paragraph $doc 'The rooms and their areas:' | Out-Null
    $placeholder = Add-Paragraph $doc 'table'
    $table = $doc.Tables.Add($placeholder.Range, 3, 2)
    $table.Cell(1, 1).Range.Text = 'Room'
    $table.Cell(1, 2).Range.Text = 'Area'
    $table.Cell(2, 1).Range.Text = 'Loading bay'
    $table.Cell(2, 2).Range.Text = '48 square metres'
    $table.Cell(3, 1).Range.Text = 'Office'
    $table.Cell(3, 2).Range.Text = '17 square metres'
    Add-Paragraph $doc 'Условия аренды' $wdStyleHeading1 | Out-Null
    Add-Paragraph $doc 'Арендатор обязуется содержать помещение в чистоте и возвращать ключи от склада управляющему.' | Out-Null
    $first = Add-Paragraph $doc 'Запрещено курить на складе'
    $second = Add-Paragraph $doc 'Запрещено хранить легковоспламеняющиеся жидкости'
    $doc.Range($first.Range.Start, $second.Range.End).ListFormat.ApplyBulletDefault()
    Add-Paragraph $doc 'Signatures' $wdStyleHeading1 | Out-Null
    $signed = Add-Paragraph $doc 'Both parties sign two copies of this lease.'

    # A footnote on the rent.
    $mark = $doc.Range($rent.Range.End - 2, $rent.Range.End - 2)
    $doc.Footnotes.Add($mark, [Type]::Missing, 'Late payment adds a penalty of two percent.') | Out-Null

    # A text box, which Word stores twice: once as a drawing, once as VML.
    $box = $doc.Shapes.AddTextbox($msoTextOrientationHorizontal, 300, 500, 180, 60, $signed.Range)
    $box.TextFrame.TextRange.Text = 'Signed copies are kept by the landlord.'

    # A comment, which is not the document's text.
    $doc.Comments.Add($doc.Range($signed.Range.Start, $signed.Range.Start + 4), 'Check the names before signing.') | Out-Null

    # A header, repeated on every page: not the document's text either.
    $doc.Sections(1).Headers($wdHeaderFooterPrimary).Range.Text = 'Confidential draft'

    if ($numbered) {
        # Headings numbered by the outline list linked to the heading styles,
        # as contracts number their clauses.
        $template = $word.ListGalleries($wdOutlineNumberGallery).ListTemplates(5)
        foreach ($paragraph in $doc.Paragraphs) {
            if ($paragraph.OutlineLevel -le 2) {
                $paragraph.Range.ListFormat.ApplyListTemplateWithLevel($template, $true, 0, 2) | Out-Null
            }
        }
    }

    # A tracked change: "monthly" deleted, "quarterly" inserted.
    $doc.TrackRevisions = $true
    $find = $doc.Content
    $find.Find.Execute('monthly') | Out-Null
    $find.Delete() | Out-Null
    $find.InsertAfter('quarterly ')
    $doc.TrackRevisions = $false

    $doc.TablesOfContents.Add($toc.Range, $true, 1, 2) | Out-Null
    # Word signs comments, revisions and properties with the signed-in
    # account, whatever the user name says; its own "remove personal
    # information" writes "Author" instead.
    $doc.RemovePersonalInformation = $true
    $doc
}

try {
    $doc = New-Lease $false
    $doc.SaveAs2((Join-Path $Out 'lease.docx'), $wdFormatXMLDocument)
    # The same document under a password: an encrypted package, not a zip.
    $doc.SaveAs2((Join-Path $Out 'locked.docx'), $wdFormatXMLDocument, $false, 'tile-grout-7')
    # And in the old binary format, then under the name of the new one - the
    # way files are renamed. Word refuses to write it under that name itself.
    $doc.SaveAs2((Join-Path $Out 'old-format.doc'), $wdFormatDocument97)
    $doc.Close($false)
    Move-Item -Force (Join-Path $Out 'old-format.doc') (Join-Path $Out 'old-format.docx')

    $doc = New-Lease $true
    $doc.SaveAs2((Join-Path $Out 'numbered.docx'), $wdFormatXMLDocument)
    $doc.Close($false)
} finally {
    # A document left open by a failure would keep Word running and its
    # file locked.
    foreach ($open in @($word.Documents)) { $open.Close($false) }
    $word.UserName = $user[0]
    $word.UserInitials = $user[1]
    $word.UserAddress = $user[2]
    $word.Quit()
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($word) | Out-Null
}
