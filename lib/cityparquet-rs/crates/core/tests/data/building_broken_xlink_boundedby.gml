<?xml version="1.0" encoding="utf-8"?>
<!-- hand-authored CityParquet test fixture (CG-1 robustness). -->
<!-- A Building with NO lodNSolid whose only boundedBy surface references a     -->
<!-- polygon by xlink:href that is defined nowhere (#missing). The reader must  -->
<!-- NOT emit a faceless MultiSurface for that LoD (it warns and drops it),     -->
<!-- leaving the Building with no geometry rather than an empty one. -->
<CityModel xmlns:xlink="http://www.w3.org/1999/xlink"
           xmlns:bldg="http://www.opengis.net/citygml/building/2.0"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns="http://www.opengis.net/citygml/2.0">
	<cityObjectMember>
		<bldg:Building gml:id="BB">
			<bldg:boundedBy>
				<bldg:WallSurface gml:id="ws">
					<bldg:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember xlink:href="#missing"/>
						</gml:MultiSurface>
					</bldg:lod2MultiSurface>
				</bldg:WallSurface>
			</bldg:boundedBy>
		</bldg:Building>
	</cityObjectMember>
</CityModel>
